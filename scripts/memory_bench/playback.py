#!/usr/bin/env python3
"""A recorded response over the production Codex stdio protocol; never a real model."""
from __future__ import annotations

import hashlib
import json
import os
import sys
import time
from pathlib import Path


def send(value):
    print(json.dumps(value), flush=True)


def main():
    if sys.argv[1:] == ["--version"]:
        print("Smaller Talk benchmark playback protocol 1")
        return
    if sys.argv[1:] == ["login", "status"]:
        print("Benchmark playback; no account or model")
        return
    if sys.argv[1:] != ["app-server", "--stdio"]:
        raise ValueError("Unsupported playback invocation")
    config = json.loads(Path(os.environ["SMALLER_TALK_BENCH_FIXTURE"]).read_text())
    response = config["response"]
    thread = "benchmark-playback-thread"
    for line in sys.stdin:
        request = json.loads(line)
        method, params = request.get("method"), request.get("params", {})
        result = {}
        if method == "initialize":
            pass
        elif method == "initialized":
            continue
        elif method == "model/list":
            result = {"data": [{"model": "benchmark-playback", "displayName": "Benchmark playback",
                                "isDefault": True, "supportedReasoningEfforts": [{"reasoningEffort": "low"}]}],
                      "nextCursor": None}
        elif method == "skills/list":
            result = {"data": [{"cwd": cwd, "skills": [], "errors": []} for cwd in params["cwds"]]}
        elif method in {"thread/start", "thread/resume"}:
            result = {"thread": {"id": thread}}
        elif method == "turn/start":
            prompt = "\n".join(item.get("text", "") for item in params["input"])
            if config["prompt"] not in prompt:
                raise ValueError("Playback did not receive the expected prompt")
            send({"id": request["id"], "result": {"turn": {"id": "benchmark-turn"}}})
            send({"method": "item/started", "params": {"threadId": thread, "item": {"type": "agentMessage", "id": "answer"}}})
            started = time.monotonic()
            max_lateness = 0.0
            chunks = [response[i:i + config["chunk_chars"]] for i in range(0, len(response), config["chunk_chars"])]
            for index, chunk in enumerate(chunks):
                time.sleep(max(0, started + index * config["chunk_interval_s"] - time.monotonic()))
                max_lateness = max(max_lateness, time.monotonic() - started - index * config["chunk_interval_s"])
                send({"method": "item/agentMessage/delta", "params": {"threadId": thread, "itemId": "answer", "delta": chunk}})
            evidence = {"pid": os.getpid(), "response_sha256": hashlib.sha256(response.encode()).hexdigest(),
                        "chunks": len(chunks), "stream_elapsed_s": time.monotonic() - started,
                        "max_schedule_lateness_s": max_lateness}
            Path(config["evidence_directory"], f"playback-{os.getpid()}.json").write_text(json.dumps(evidence))
            send({"method": "item/completed", "params": {"threadId": thread,
                  "item": {"type": "agentMessage", "phase": "final_answer", "id": "answer", "text": response}}})
            send({"method": "turn/completed", "params": {"threadId": thread, "turn": {"status": "completed"}}})
            continue
        else:
            if "id" in request:
                send({"id": request["id"], "error": {"code": -32601, "message": "Unsupported playback method"}})
            continue
        send({"id": request["id"], "result": result})


if __name__ == "__main__":
    main()

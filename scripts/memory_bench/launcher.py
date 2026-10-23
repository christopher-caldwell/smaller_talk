"""One-command setup for a disposable Smaller Talk release capture."""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import plistlib
import sqlite3
import subprocess
import uuid
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path

from .core import write_json
from .history import save_results


def configure_database(path: Path, prompt: str, model: str | None, effort: str | None) -> None:
    """Prepare only a new benchmark database; the prompt remains unsent."""
    chat_id = str(uuid.uuid4())
    execution = {"provider": "codex", "model": model, "effort": effort, "plan_mode": False}
    selection = len(prompt.encode("utf-8"))
    session = {
        "open_tabs": [chat_id], "active_tab": chat_id, "sidebar_collapsed": False,
        "scroll_positions": {},
        "new_chat": {
            "id": chat_id,
            "preferences": {"execution": execution, "project_id": None,
                            "binding_workspace": None, "new_session": False},
            "draft": {"text": prompt, "skills": [], "attachments": [],
                      "selection_start": selection, "selection_end": selection},
            "submitted": None,
        },
    }
    settings = {
        "providers": {"codex": {"enabled": True}, "claude": {"enabled": False}},
        "default_execution": execution, "automatic_shortening": False,
    }
    with closing(sqlite3.connect(path)) as database, database:
        database.execute("INSERT INTO global_settings (id, payload) VALUES (1, ?)", (json.dumps(settings),))
        database.execute("INSERT INTO app_state (id, payload) VALUES (1, ?)", (json.dumps(session),))


def run(args: argparse.Namespace) -> int:
    if platform.system() != "Darwin":
        raise ValueError("The desktop benchmark launcher currently requires macOS")
    if not math.isfinite(args.interval) or args.interval < 0.05:
        raise ValueError("Sampling interval must be finite and at least 0.05 seconds")
    if args.duration is not None and (not math.isfinite(args.duration) or args.duration <= 0):
        raise ValueError("Duration must be finite and positive")
    if not args.label.strip():
        raise ValueError("Capture label must not be empty")
    repo = Path(__file__).resolve().parents[2]
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    base = (args.output or repo / ".dev/memory-bench" / f"{stamp}-smaller-talk").expanduser().resolve()
    if base.exists():
        raise FileExistsError(f"Output directory already exists: {base}")
    print("Building Smaller Talk in release mode before measurement...", flush=True)
    environment = {**os.environ, "SQLX_OFFLINE": "true"}
    environment.setdefault("CARGO_BUILD_JOBS", "2")
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "smaller-talk-desktop"],
                   cwd=repo, env=environment, check=True)
    binary = repo / "target/release/smaller-talk-desktop"
    base.mkdir(parents=True, mode=0o700)
    data, workspace = base / "data", base / "workspace"
    data.mkdir()
    workspace.mkdir()
    environment["SMALLER_TALK_DATA_DIR"] = str(data)
    print(f"Benchmark files: {base}", flush=True)
    with (base / "setup.log").open("w") as log:
        for command in ("db-create", "db-migrate"):
            subprocess.run([str(binary), command], cwd=workspace, env=environment,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
    template = json.loads((repo / "benchmarks/workloads/codex-manual-v1.json").read_text())
    prompt = template["prompt"]
    configure_database(data / "chats.sqlite3", prompt, args.model, args.effort)
    template["required_operator_record"] = {
        "model": args.model or "Provider default; confirm in the app before sending",
        "reasoning_effort": args.effort or "Provider default; confirm in the app before sending",
        "provider_version": "Inherited installed Codex; record version for controlled comparisons",
        "workspace": str(workspace),
        "window": "Default app bounds; confirm display scale and geometry",
        "configuration": "Fresh DB; Codex enabled; Claude disabled; shortening off; inherited host provider configuration",
    }
    template["execution"] = "Interactive native UI; original prompt prefilled but never automatically submitted"
    template["starting_state"] = "Fresh unsent conversation with original prompt prefilled; fresh disposable database and no attachments"
    write_json(base / "workload.json", template)
    (base / "workload-prompt.txt").write_text(prompt)
    bundle = base / "Smaller Talk Benchmark.app"
    executable = bundle / "Contents/MacOS/SmallerTalk"
    executable.parent.mkdir(parents=True)
    executable.symlink_to(binary)
    with (bundle / "Contents/Info.plist").open("wb") as info:
        plistlib.dump({
            "CFBundleIdentifier": "com.smallertalk.memory-benchmark",
            "CFBundleName": "Smaller Talk Benchmark", "CFBundleDisplayName": "Smaller Talk Benchmark",
            "CFBundleExecutable": "SmallerTalk", "CFBundlePackageType": "APPL",
            "CFBundleVersion": "1", "NSHighResolutionCapable": True,
        }, info)
    with (base / "app.log").open("w") as log:
        app = subprocess.Popen([str(executable)], cwd=workspace, env=environment, stdin=subprocess.DEVNULL,
                               stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    launch = {"app_pid": app.pid, "binary": str(binary), "bundle": str(bundle), "data": str(data),
              "workspace": str(workspace), "capture": str(base / "capture"),
              "prompt_submitted_automatically": False, "context": args.context}
    write_json(base / "launch.json", launch)
    print("A fresh Smaller Talk window opens with the original benchmark prompt prefilled.", flush=True)
    print("1. Confirm model/sign-in; type idle here and wait about 30 seconds.", flush=True)
    print("2. Type work here, then press Send in Smaller Talk.", flush=True)
    print("3. When the response finishes, type post; wait about 30 seconds, then stop.", flush=True)
    print("snap takes an optional footprint checkpoint. The app stays open after capture.", flush=True)
    from .cli import main

    command = ["record", "--target", "smaller-talk", "--pid", str(app.pid),
               "--label", args.label, "--output", str(base / "capture"),
               "--workload", str(base / "workload.json"), "--context", args.context,
               "--interval", str(args.interval), "--phase", "launch"]
    if args.duration is not None:
        command.extend(["--duration", str(args.duration)])
    result = main(command)
    launch["capture_exit_code"] = result
    launch["app_left_open"] = app.poll() is None
    write_json(base / "launch.json", launch)
    print(f"Results: {base / 'capture/report.md'}", flush=True)
    if (base / "capture/manifest.json").exists():
        print(f"Git results: {save_results(base)}", flush=True)
    print("Close the benchmark window when finished; its disposable data remains here.", flush=True)
    return result

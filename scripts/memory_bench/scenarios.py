"""Unattended native Smaller Talk scenarios, independent repeats, and evidence checks."""
from __future__ import annotations

import hashlib
import json
import math
import os
import platform
import random
import shutil
import signal
import sqlite3
import statistics
import subprocess
import time
import uuid
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path

from .core import Capture, command_metadata, file_sha256, read_processes, utc_now, write_json
from .history import save_results
from .launcher import configure_database
from .macos import FootprintReader
from .report import save_report

PROTOCOL = 1
HISTORY_ID = str(uuid.uuid5(uuid.NAMESPACE_URL, "smaller-talk/benchmark/history-v1"))


class ScenarioFailure(RuntimeError):
    pass


class HarnessFailure(RuntimeError):
    pass


class AppEvents:
    def __init__(self, path: Path, app: subprocess.Popen, capture: Capture | None = None):
        self.path, self.app, self.capture = path, app, capture
        self.offset = 0
        self.events: list[dict] = []
        self.request = 0

    def read(self):
        if not self.path.exists():
            return []
        values = []
        with self.path.open() as stream:
            stream.seek(self.offset)
            while True:
                position = stream.tell()
                line = stream.readline()
                if not line or not line.endswith("\n"):
                    self.offset = position
                    break
                try:
                    value = json.loads(line)
                    if not isinstance(value, dict) or "event" not in value:
                        raise ValueError("Missing application event")
                except ValueError as error:
                    raise HarnessFailure(f"Malformed application evidence: {error}") from error
                values.append(value)
                self.events.append(value)
                if self.capture:
                    self.capture.emit("application_event", application=value)
        for value in values:
            if value["event"] == "error":
                raise ScenarioFailure(str(value["data"]))
        return values

    def wait(self, event: str, timeout: float, request: int | None = None) -> dict:
        deadline = time.monotonic() + timeout
        while True:
            self.read()
            found = next((value for value in self.events if value["event"] == event
                          and (request is None or value["data"].get("request") == request)), None)
            if found:
                return found
            if self.app.poll() is not None:
                if event == "boot_waiting":
                    raise HarnessFailure(f"Executable did not start benchmark protocol {PROTOCOL} (exit {self.app.returncode}); see app.log")
                raise ScenarioFailure(f"Smaller Talk exited ({self.app.returncode}) before {event}")
            if time.monotonic() >= deadline:
                raise ScenarioFailure(f"Timed out waiting for {event}")
            time.sleep(0.04)

    def send(self, command: str, **data) -> int:
        self.request += 1
        try:
            self.app.stdin.write(json.dumps({"command": command, "id": self.request, **data}) + "\n")
            self.app.stdin.flush()
        except (OSError, ValueError) as error:
            raise ScenarioFailure(f"Could not send benchmark command: {error}") from error
        return self.request

    def action(self, command: str, timeout: float, **data) -> dict:
        request = self.send(command, **data)
        return self.wait("view_rendered", timeout, request)

    def settle(self, seconds: float):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.read()
            if self.app.poll() is not None:
                raise ScenarioFailure("Smaller Talk exited during settling")
            if self.capture and self.capture.collector_failure:
                raise HarnessFailure(self.capture.collector_failure)
            time.sleep(min(0.04, max(0, deadline - time.monotonic())))


def seed_history(database: Path, response: str) -> dict:
    """Setup happens before measurement; measured navigation uses normal UI/service reads."""
    with closing(sqlite3.connect(database)) as connection, connection:
        connection.execute("PRAGMA foreign_keys=ON")
        connection.execute("INSERT INTO chats(id,title,created_at) VALUES(?,?,?)",
                           (HISTORY_ID, "Fixed benchmark history", 1_700_000_000_000))
        data = []
        for index in range(200):
            body = response * 4 if index % 2 else f"Explain range merging, example {index // 2}."
            data.append((str(uuid.uuid5(uuid.NAMESPACE_URL, f"benchmark-history-v1/{index}")),
                         HISTORY_ID, "assistant" if index % 2 else "user", body,
                         1_700_000_000_000 + index))
        connection.executemany("INSERT INTO messages(id,chat_id,role,body,created_at) VALUES(?,?,?,?,?)", data)
    return {"chat_id": HISTORY_ID, "messages": len(data),
            "body_bytes": sum(len(row[3].encode()) for row in data),
            "sha256": hashlib.sha256(json.dumps(data, ensure_ascii=False).encode()).hexdigest()}


def seed_workspace(workspace: Path, base: Path) -> str:
    """Stop Git discovery at an identical local fixture repository, not the source checkout."""
    template = base / "empty-git-template"
    template.mkdir()
    environment = {**os.environ, "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
                   "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z"}
    commands = [
        ["git", "init", "--quiet", "--initial-branch=benchmark", f"--template={template}", str(workspace)],
        ["git", "-C", str(workspace), "config", "core.hooksPath", "/dev/null"],
        ["git", "-C", str(workspace), "config", "core.fsmonitor", "false"],
        ["git", "-C", str(workspace), "-c", "user.name=Smaller Talk Benchmark", "-c", "user.email=benchmark@example.invalid",
         "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "Fixed benchmark workspace"],
    ]
    with (base / "workspace-setup.log").open("w") as log:
        for command in commands:
            subprocess.run(command, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=15)
    return subprocess.run(["git", "-C", str(workspace), "rev-parse", "HEAD"], capture_output=True,
                          text=True, check=True, timeout=5).stdout.strip()


def verify_database(database: Path, chat_id: str, prompt: str, expected: str | None) -> dict:
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as connection:
        turns = connection.execute("SELECT status,error,settings FROM turns WHERE chat_id=?", (chat_id,)).fetchall()
        messages = connection.execute("SELECT role,body FROM messages WHERE chat_id=? ORDER BY created_at,id", (chat_id,)).fetchall()
        finals = connection.execute("SELECT f.text FROM final_answers f JOIN messages m ON m.id=f.message_id WHERE m.chat_id=?", (chat_id,)).fetchall()
        pending = connection.execute("SELECT count(*) FROM message_queue WHERE chat_id=?", (chat_id,)).fetchone()[0]
    users = [body for role, body in messages if role == "user"]
    response = "\n\n".join(body for role, body in messages if role == "assistant")
    if len(turns) != 1 or turns[0][0] != "completed" or turns[0][1] or pending:
        raise ScenarioFailure(f"Unexpected persisted execution state: turns={turns}, queue={pending}")
    if users != [prompt] or not response.strip():
        raise ScenarioFailure("Persisted prompt or assistant response is missing/incorrect")
    if expected is not None and (response != expected or [row[0] for row in finals] != [expected]):
        raise ScenarioFailure("Playback response/final-answer bytes differ from the fixture")
    return {"chat_id": chat_id, "completed_turns": 1, "pending_queue_entries": pending,
            "settings": json.loads(turns[0][2]), "response_bytes": len(response.encode()),
            "response_sha256": hashlib.sha256(response.encode()).hexdigest(),
            "prompt_sha256": hashlib.sha256(prompt.encode()).hexdigest(),
            "fixture_exact_match": expected is not None, "final_answers": len(finals), "response": response}


def children(capture: Capture, rows: dict) -> list:
    with capture.lock:
        selected = capture.tree.select(rows)
        return [row for row in rows.values() if row.identity in selected and row.identity != capture.tree.root]


def await_backend_exit(capture: Capture, events: AppEvents, timeout: float = 15):
    deadline = time.monotonic() + timeout
    clean = 0
    while time.monotonic() < deadline:
        events.read()
        current = children(capture, read_processes())
        clean = clean + 1 if not current else 0
        if clean >= 2:
            capture.emit("backend_exit_confirmed", method="two process-tree snapshots; polling may miss brief children")
            return
        events.settle(0.25)
    raise ScenarioFailure("Owned execution processes remained after completion")


def cleanup(app: subprocess.Popen, events: AppEvents | None, capture: Capture | None) -> dict:
    forced = False
    if app.poll() is None:
        try:
            if events:
                events.send("quit")
            else:
                app.stdin.close()
            app.wait(timeout=10)
        except (OSError, ValueError, ScenarioFailure, subprocess.TimeoutExpired):
            forced = True
            # Only the process group created by this launcher, never other app instances.
            try:
                os.killpg(app.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait(timeout=5)
    if app.stdin and not app.stdin.closed:
        app.stdin.close()
    remaining = children(capture, read_processes()) if capture else []
    # Reparented children are killed only with their already-pinned process identity.
    for process in remaining:
        fresh = read_processes().get(process.pid)
        if fresh and fresh.identity == process.identity:
            try:
                os.kill(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
    if remaining:
        time.sleep(0.3)
    residual = children(capture, read_processes()) if capture else []
    return {"app_exit_code": app.returncode, "forced_app_shutdown": forced,
            "terminated_residual_children": [p.pid for p in remaining],
            "remaining_owned_processes": [p.pid for p in residual]}


def bundle(binary: Path, base: Path) -> Path:
    import plistlib
    directory = base / "Smaller Talk Benchmark.app"
    executable = directory / "Contents/MacOS/SmallerTalk"
    executable.parent.mkdir(parents=True)
    executable.symlink_to(binary)
    with (directory / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleIdentifier": "com.smallertalk.memory-benchmark", "CFBundleName": "Smaller Talk Benchmark",
                      "CFBundleExecutable": "SmallerTalk", "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
                      "NSHighResolutionCapable": True}, stream)
    return executable


def trial(base: Path, binary: Path, kind: str, args, prompt: str, response: str, build: str, pair: int) -> dict:
    base.mkdir(mode=0o700)
    data, workspace = base / "data", base / "workspace"
    data.mkdir()
    workspace.mkdir()
    workspace_revision = seed_workspace(workspace, base)
    environment = {**os.environ, "SMALLER_TALK_DATA_DIR": str(data)}
    model, effort = ("benchmark-playback", "low") if kind == "playback" else (args.model, args.effort)
    provider = None
    if kind == "playback":
        fixture = base / "codex-playback"
        shutil.copyfile(Path(__file__).with_name("playback.py"), fixture)
        fixture.chmod(0o700)
        environment["SMALLER_TALK_CODEX_BIN"] = str(fixture)
        fixture_config = base / "playback.json"
        write_json(fixture_config, {"prompt": prompt, "response": response, "chunk_chars": 80,
                                   "chunk_interval_s": 0.08, "evidence_directory": str(base)})
        environment["SMALLER_TALK_BENCH_FIXTURE"] = str(fixture_config)
    else:
        provider = args.codex_binary or os.environ.get("SMALLER_TALK_CODEX_BIN") or shutil.which("codex")
        if not provider:
            raise HarnessFailure("Codex CLI not found; specify --codex-binary or use --offline")
        provider = str(Path(provider).expanduser().resolve())
        environment["SMALLER_TALK_CODEX_BIN"] = provider
    with (base / "setup.log").open("w") as log:
        for command in ("db-create", "db-migrate"):
            subprocess.run([str(binary), command], env=environment, cwd=workspace,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=60)
    database = data / "chats.sqlite3"
    configure_database(database, prompt, model, effort)
    history = seed_history(database, response) if kind == "playback" else None
    workload = {"protocol": PROTOCOL, "scenario": kind, "build": build, "pair": pair,
                "prompt_sha256": hashlib.sha256(prompt.encode()).hexdigest(),
                "expected_response_sha256": hashlib.sha256(response.encode()).hexdigest() if kind == "playback" else None,
                "model": model, "effort": effort, "settle_s": args.settle_seconds,
                "retention_cycles": args.cycles if kind == "playback" else 0, "history": history,
                "provider_binary": provider, "provider_version": command_metadata([provider, "--version"]) if provider else None,
                "provider_sha256": file_sha256(Path(provider)) if provider else None,
                "provider_configuration": "Inherited host configuration and MCP helpers" if kind == "live" else "Local protocol playback; no model or real CLI",
                "workspace_revision": workspace_revision,
                "viewport": "1120x780 logical window content; display scale inherited",
                "launch_definition": "fresh process and database; OS caches not flushed"}
    write_json(base / "workload.json", workload)
    write_json(base / "benchmark.json", {"protocol": PROTOCOL})
    result = {"kind": kind, "build": build, "pair": pair, "directory": str(base),
              "outcome": "incomplete", "binary_sha256": file_sha256(binary)}
    capture = None
    events = None
    app = None
    reason = "Scenario did not finish"
    print(f"  {base.name}: {kind} / {build}", flush=True)
    try:
        with (base / "app.log").open("w") as log:
            app = subprocess.Popen([str(bundle(binary, base)), "benchmark", str(base)], env=environment,
                                   cwd=workspace, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
                                   text=True, start_new_session=True)
        result["app_pid"] = app.pid
        events = AppEvents(base / "app-events.jsonl", app)
        events.wait("boot_waiting", args.timeout)
        capture = Capture(base / "capture", app.pid, [], "smaller-talk", f"{args.label}: {kind}/{build}",
                          args.context, args.interval, base / "workload.json", footprint_reader=FootprintReader())
        events.capture = capture
        capture.manifest["launch_gate"] = "attached before application initialization; executable loading precedes attachment"
        capture.mark("launch")
        capture.start()
        events.send("start")
        ready = events.wait("ui_ready", args.timeout)
        result["initialization_to_ready_s"] = ready["elapsed_s"] - next(e["elapsed_s"] for e in events.events if e["event"] == "initializing")
        capture.mark("settle", "Wait for initial provider discovery to exit, then a fixed warmup")
        await_backend_exit(capture, events)
        events.settle(args.settle_seconds)
        capture.mark("idle", "UI ready, fixed settling window")
        events.settle(args.settle_seconds)
        capture.mark("work", "Automated submission through the normal UI send handler")
        events.send("send")
        completion = events.wait("rendered_complete", args.timeout)
        await_backend_exit(capture, events)
        submitted = next(e for e in events.events if e["event"] == "submitted")
        result["submit_to_frame_complete_s"] = completion["elapsed_s"] - submitted["elapsed_s"]
        capture.mark("post", "Completed frame and owned execution process cleanup verified")
        evidence = verify_database(database, completion["data"]["chat_id"], prompt, response if kind == "playback" else None)
        (base / "response.md").write_text(evidence.pop("response"))
        write_json(base / "completion.json", evidence)
        result["completion"] = evidence
        events.settle(args.settle_seconds)
        if kind == "live":
            # A real live run must have captured the actual selected CLI, not just an app-only total.
            trace = [json.loads(line) for line in (capture.output / "events.jsonl").read_text().splitlines()]
            observed = [p for event in trace if event["type"] == "sample" for p in event["processes"]
                        if p["pid"] != app.pid and (Path(p["executable"]).resolve() == Path(provider)
                                                    or p.get("backend") == "codex")]
            if not observed:
                raise HarnessFailure("The live Codex CLI was not observed in memory samples; full execution cost is unverified")
            result["real_codex_observed_pids"] = sorted({p["pid"] for p in observed})
            result["real_codex_observed_executables"] = sorted({p["executable"] for p in observed})
        else:
            playback = list(base.glob("playback-*.json"))
            if len(playback) != 1:
                raise HarnessFailure("Missing or duplicate playback process evidence")
            result["playback"] = json.loads(playback[0].read_text())
            events.action("close", args.timeout)
            capture.mark("closed-baseline", "Closed response before first history load")
            events.settle(args.settle_seconds)
            for cycle in range(args.cycles):
                capture.mark(f"history-{cycle + 1}", "Open fixed history and render both ends")
                opened = events.action("open", args.timeout, chat_id=HISTORY_ID)
                if opened["data"].get("message_count") != history["messages"] or opened["data"].get("active_chat_id") != HISTORY_ID:
                    raise HarnessFailure("History frame did not contain the expected loaded conversation")
                top = events.action("scroll", args.timeout, end=False)
                if top["data"].get("scroll_item") != 0:
                    raise HarnessFailure("History did not render its first message")
                events.settle(args.settle_seconds)
                end = events.action("scroll", args.timeout, end=True)
                if end["data"].get("scroll_item", 0) <= 0:
                    raise HarnessFailure("History did not scroll toward its final message")
                events.settle(args.settle_seconds)
                closed = events.action("close", args.timeout)
                if closed["data"].get("active_chat_id") is not None:
                    raise HarnessFailure("Retention cycle did not return to the closed-tab state")
                capture.mark(f"retained-{cycle + 1}", "Same closed-tab state; fixed settling window")
                events.settle(args.settle_seconds)
        capture.manifest["workload_verification"] = "verified_automated"
        capture.manifest["limitations"] = [limit for limit in capture.manifest["limitations"] if not limit.startswith("Manual capture")]
        result["outcome"], reason = "completed", "Scenario, persisted response, completed frame, and execution cleanup verified"
    except KeyboardInterrupt:
        reason = "Interrupted by operator"
        raise
    except (OSError, ValueError, RuntimeError, sqlite3.Error, subprocess.SubprocessError) as error:
        reason = f"{type(error).__name__}: {error}"
        result["outcome"] = "product_failure" if isinstance(error, ScenarioFailure) else "harness_invalid"
        print(f"    {reason}", flush=True)
    finally:
        try:
            if capture:
                capture.finish(result["outcome"], reason)
                report = save_report(capture.output)
                if report["outcome"] != result["outcome"]:
                    reason = report["reason"]
                result["outcome"] = report["outcome"]
                result["metrics"] = trial_metrics(report)
                inadequate = capture_issues(report, kind, args.cycles)
                if inadequate and result["outcome"] == "completed":
                    result["outcome"] = "harness_invalid"
                    reason = f"Insufficient or failed memory samples: {inadequate}"
        except (OSError, ValueError, RuntimeError) as error:
            result["outcome"], reason = "harness_invalid", f"Report/collector failure: {error}"
        finally:
            if app:
                result["cleanup"] = cleanup(app, events, capture)
                if (result["cleanup"]["forced_app_shutdown"] or result["cleanup"]["terminated_residual_children"]
                        or result["cleanup"]["app_exit_code"] != 0) and result["outcome"] == "completed":
                    result["outcome"], reason = "product_failure", "App did not shut down cleanly"
        if capture and capture.finished:
            capture.manifest["cleanup"] = result.get("cleanup")
            capture.assess(result["outcome"], reason)
            save_report(capture.output)
        result["reason"] = reason
        write_json(base / "trial.json", result)
    return result


def capture_issues(report: dict, kind: str, cycles: int) -> list[str]:
    expected = {"idle", "work", "post"}
    if kind == "playback":
        expected.add("closed-baseline")
        expected.update(f"{phase}-{cycle + 1}" for cycle in range(cycles) for phase in ("history", "retained"))
    phases = {phase["name"]: phase for phase in report["phases"]}
    issues = [f"missing phase {name}" for name in sorted(expected - phases.keys())]
    for name in sorted(expected & phases.keys()):
        phase = phases[name]
        if phase["metric_samples"] < 2 or phase["footprint_samples"]["total_footprint_kib"] < 2:
            issues.append(f"insufficient samples in {name}")
    if report["sample_errors"] or report["malformed_records"]:
        issues.append("sampling errors or malformed records")
    return issues


def trial_metrics(report: dict) -> dict:
    metrics = {}
    for phase in report["phases"]:
        for prefix, group in (("rss", phase["metrics_kib"]), ("footprint", phase["footprint_metrics_kib"])):
            app_key = "native_rss_kib" if prefix == "rss" else "native_footprint_kib"
            total_key = "total_rss_kib" if prefix == "rss" else "total_footprint_kib"
            for role, key in (("app", app_key), ("total", total_key)):
                values = group[key]
                if values:
                    metrics[f"{phase['name']}.{role}_{prefix}_median_mib"] = values["median"] / 1024
                    metrics[f"{phase['name']}.{role}_{prefix}_sampled_max_mib"] = values["sampled_max"] / 1024
    return metrics


def paired_comparison(trials: list[dict], seed: int) -> dict:
    groups = {}
    for result in trials:
        if result["kind"] == "playback":
            groups.setdefault(result["pair"], {})[result["build"]] = result
    complete = [pair for pair in groups.values() if set(pair) == {"baseline", "candidate"}
                and all(result["outcome"] == "completed" for result in pair.values())]
    metrics = {}
    if complete:
        keys = set.intersection(*(set(pair[build]["metrics"]) for pair in complete for build in ("baseline", "candidate")))
        rng = random.Random(seed)
        for key in sorted(keys):
            differences = [pair["candidate"]["metrics"][key] - pair["baseline"]["metrics"][key] for pair in complete]
            # Explicitly exploratory bootstrap. Few pairs do not establish a release gate.
            boot = sorted(statistics.mean(rng.choices(differences, k=len(differences))) for _ in range(2000))
            metrics[key] = {"paired_mean_delta_mib": statistics.mean(differences),
                            "paired_deltas_mib": differences,
                            "exploratory_bootstrap_95_percent_interval_mib": [boot[49], boot[1949]] if len(differences) >= 5 else None}
    return {"complete_pairs": len(complete), "attempted_pairs": len(groups), "metrics": metrics,
            "verdict": "not_assessed", "note": "All attempts retained. Incomplete pairs excluded explicitly; fewer than five pairs have no interval. Bootstrap intervals are exploratory, not release gates."}


def suite_report(base: Path, trials: list[dict], args, metadata: dict):
    groups = {}
    for trial_result in trials:
        group = groups.setdefault(f"{trial_result['kind']}/{trial_result['build']}", [])
        group.append(trial_result)
    outcome = "completed" if trials and all(t["outcome"] == "completed" for t in trials) else "failed"
    if metadata.get("interrupted") or len(trials) < metadata.get("expected_attempts", len(trials)):
        outcome = "incomplete"
    summary = {**metadata, "trials": trials, "groups": {}, "outcome": outcome,
               "performance_verdict": "not_assessed", "comparison": paired_comparison(trials, args.seed)}
    lines = ["# Smaller Talk automated memory benchmark", "",
             f"Host context: **{args.context}**. Outcome: **{summary['outcome']}**. Performance verdict: **not assessed**.", "",
             "Playback measures a fixed recorded workload through the native app. Live measures the app plus its real Codex CLI and owned helpers. These are separate results.", "",
             "Footprint totals sum libproc per-process accounting; they are not deduplicated group physical memory. RSS is separate. Peaks are simultaneous sampled totals.", "",
             "| Scenario / build | Completed / attempted | Idle footprint MiB | Work sampled max footprint MiB | Work sampled max RSS MiB | Post footprint MiB |",
             "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for name, attempts in groups.items():
        valid = [t for t in attempts if t["outcome"] == "completed"]
        keys = set.union(*(set(t.get("metrics", {})) for t in valid)) if valid else set()
        aggregated = {key: {"median_across_runs": statistics.median([t["metrics"][key] for t in valid if key in t["metrics"]]),
                            "individual_runs": [t["metrics"].get(key) for t in valid]} for key in sorted(keys)}
        summary["groups"][name] = {"attempted": len(attempts), "completed": len(valid), "metrics": aggregated}
        def number(key):
            value = aggregated.get(key)
            return f"{value['median_across_runs']:.2f}" if value else "unavailable"
        lines.append(f"| {name} | {len(valid)}/{len(attempts)} | {number('idle.total_footprint_median_mib')} | "
                     f"{number('work.total_footprint_sampled_max_mib')} | {number('work.total_rss_sampled_max_mib')} | {number('post.total_footprint_median_mib')} |")
    if args.cycles:
        lines.extend(["", "App-only fixed-history cycles (footprint MiB, median of run summaries):", "",
                      "| Build | App before history, closed | App history sampled max | App closed after first cycle | App closed after last cycle |",
                      "| --- | ---: | ---: | ---: | ---: |"])
        for name, group in summary["groups"].items():
            if not name.startswith("playback/"):
                continue
            metrics = group["metrics"]
            def value(key):
                return f"{metrics[key]['median_across_runs']:.2f}" if key in metrics else "unavailable"
            peaks = [data["median_across_runs"] for key, data in metrics.items()
                     if key.startswith("history-") and key.endswith("app_footprint_sampled_max_mib")]
            peak = f"{max(peaks):.2f}" if peaks else "unavailable"
            lines.append(f"| {name.split('/')[1]} | {value('closed-baseline.app_footprint_median_mib')} | {peak} | "
                         f"{value('retained-1.app_footprint_median_mib')} | {value(f'retained-{args.cycles}.app_footprint_median_mib')} |")
    lines.extend(["", "The group table is the median of independent run summaries. Each run's app/helper breakdown, raw samples, semantic events, verified output, and cleanup evidence are retained.", "",
                  "## Attempts", ""])
    for attempt in trials:
        relative = Path(attempt["directory"]).relative_to(base)
        lines.append(f"- [{relative}]({relative}/trial.json): **{attempt['outcome']}** — {attempt['reason']}")
    if args.baseline_binary:
        comparison = summary["comparison"]
        lines.extend(["", f"Paired comparison: {comparison['complete_pairs']}/{comparison['attempted_pairs']} complete pairs. See suite.json for paired deltas and exploratory intervals."])
    lines.extend(["", "## Interpretation", "",
                  "- Busy-system runs establish scenario behavior, not a trustworthy regression result.",
                  "- Repeated samples within a run are not independent runs. Live output and remote model behavior vary.",
                  "- A fresh process/database is not a cold OS cache. Initialization timing excludes executable loading and collector attachment.",
                  "- Retained phases revisit the same closed-tab state. Cache/allocator retention is not automatically a leak.",
                  "- Polling can miss short-lived children and peaks; per-process footprint reads are sequential.",
                  "- UI completion is a GPUI post-frame callback plus database verification, not a pixel assertion.",
                  "- Provider configuration is inherited for live runs. Record matching model, CLI, MCP, and host conditions for comparisons."])
    write_json(base / "suite.json", summary)
    (base / "report.md").write_text("\n".join(lines) + "\n")
    return summary


def prepare_binaries(base: Path, repo: Path, args) -> tuple[dict, dict]:
    if not args.binary:
        print("Building release before measurement...", flush=True)
        environment = {**os.environ, "SQLX_OFFLINE": "true"}
        environment.setdefault("CARGO_BUILD_JOBS", "2")
        with (base / "build.log").open("w") as log:
            subprocess.run(["cargo", "build", "--release", "--locked", "-p", "smaller-talk-desktop"],
                           cwd=repo, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
    originals = {"candidate": Path(args.binary or repo / "target/release/smaller-talk-desktop").expanduser().resolve()}
    if args.baseline_binary:
        originals["baseline"] = Path(args.baseline_binary).expanduser().resolve()
    binaries = {}
    for name, source in originals.items():
        destination = base / name / "smaller-talk-desktop"
        destination.parent.mkdir()
        shutil.copy2(source, destination)
        binaries[name] = destination
    return binaries, originals


def run(args) -> int:
    if platform.system() != "Darwin":
        raise ValueError("Automated desktop benchmarking currently requires macOS")
    for name, value in (("interval", args.interval), ("settle seconds", args.settle_seconds), ("timeout", args.timeout)):
        if not math.isfinite(value) or value <= 0:
            raise ValueError(f"{name} must be finite and positive")
    if args.interval < 0.05 or args.settle_seconds < args.interval * 3:
        raise ValueError("Settling must cover at least three sample intervals (minimum interval 0.05s)")
    if args.repeats < 1 or args.cycles < 0 or not args.label.strip():
        raise ValueError("Use positive repeats, nonnegative cycles, and a nonempty label")
    for path in (args.binary, args.baseline_binary, args.codex_binary):
        if path and (not Path(path).expanduser().is_file() or not os.access(Path(path).expanduser(), os.X_OK)):
            raise ValueError(f"Not an executable file: {path}")
    repo = Path(__file__).resolve().parents[2]
    base = (args.output or repo / ".dev/memory-bench" / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ") + "-suite")).expanduser().resolve()
    if base.exists():
        raise FileExistsError(f"Output directory already exists: {base}")
    base.mkdir(parents=True, mode=0o700)
    metadata = {"protocol": PROTOCOL, "context": args.context, "seed": args.seed,
                "run_id": uuid.uuid4().hex, "started_at": utc_now(), "label": args.label,
                "source_revision": command_metadata(["git", "rev-parse", "HEAD"], repo),
                "source_branch": command_metadata(["git", "branch", "--show-current"], repo),
                "source_status": command_metadata(["git", "status", "--porcelain"], repo),
                "offline": args.offline, "repeat_count": args.repeats, "interval_s": args.interval,
                "settle_s": args.settle_seconds, "retention_cycles": args.cycles,
                "expected_attempts": (args.repeats + int(not args.offline)) * (2 if args.baseline_binary else 1),
                "framework_files_sha256": {str(path.relative_to(repo)): file_sha256(path)
                    for path in [*sorted((repo / "scripts/memory_bench").glob("*.py")),
                                 repo / "crates/desktop/src/benchmark.rs", repo / "crates/desktop/src/shell/benchmark.rs"]}}
    write_json(base / "suite.json", {**metadata, "outcome": "preparing"})
    print(f"Benchmark results: {base}", flush=True)
    try:
        binaries, originals = prepare_binaries(base, repo, args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        write_json(base / "suite.json", {**metadata, "outcome": "harness_invalid", "reason": str(error), "trials": []})
        (base / "report.md").write_text(f"# Smaller Talk memory benchmark\n\nPreparation failed: {error}\n\nSee build.log if a build was attempted. No performance result.\n")
        save_results(base)
        raise
    except KeyboardInterrupt:
        write_json(base / "suite.json", {**metadata, "outcome": "incomplete", "reason": "Interrupted during preparation", "trials": []})
        save_results(base)
        raise
    metadata["binaries"] = {name: {"source": str(originals[name]), "sha256": file_sha256(path)} for name, path in binaries.items()}
    prompt = (repo / "benchmarks/reference/v3/smaller_talk_workload_prompt.txt").read_text()
    response = (repo / "benchmarks/fixtures/codex-response-v1.md").read_text()
    metadata["fixture_sha256"] = hashlib.sha256(response.encode()).hexdigest()
    metadata["prompt_sha256"] = hashlib.sha256(prompt.encode()).hexdigest()
    rng = random.Random(args.seed)
    trials = []
    def attempt(directory, binary, kind, build, pair):
        try:
            return trial(directory, binary, kind, args, prompt, response, build, pair)
        except KeyboardInterrupt:
            metadata["interrupted"] = True
            if (directory / "trial.json").exists():
                trials.append(json.loads((directory / "trial.json").read_text()))
            else:
                directory.mkdir(exist_ok=True)
                interrupted = {"kind":kind,"build":build,"pair":pair,"directory":str(directory),
                               "outcome":"incomplete","reason":"Interrupted during trial preparation"}
                write_json(directory / "trial.json", interrupted)
                trials.append(interrupted)
            raise
        except (OSError, ValueError, RuntimeError, sqlite3.Error, subprocess.SubprocessError) as error:
            directory.mkdir(exist_ok=True)
            result = {"kind":kind,"build":build,"pair":pair,"directory":str(directory),
                      "outcome":"harness_invalid","reason":f"Preparation/finalization failed: {error}"}
            write_json(directory / "trial.json", result)
            print(f"  {directory.name}: {result['reason']}", flush=True)
            return result
    try:
        for pair in range(1, args.repeats + 1):
            order = list(binaries)
            rng.shuffle(order)
            for build in order:
                trials.append(attempt(base / f"playback-{pair:02d}-{build}", binaries[build], "playback", build, pair))
                suite_report(base, trials, args, metadata)
        if not args.offline:
            # One live run per build is descriptive; repeated offline trials never stand in for CLI cost.
            order = list(binaries)
            rng.shuffle(order)
            for build in order:
                trials.append(attempt(base / f"live-{build}", binaries[build], "live", build, 0))
                suite_report(base, trials, args, metadata)
    finally:
        summary = suite_report(base, trials, args, metadata)
        print(f"Git results: {save_results(base)}", flush=True)
    print(f"Report: {base / 'report.md'}", flush=True)
    print("All scenarios completed; measurements remain observational." if summary["outcome"] == "completed" else "Some attempts failed; evidence retained in the report.", flush=True)
    return 0 if summary["outcome"] == "completed" else 1

"""Manual phase commands over a background process-tree sampler."""

from __future__ import annotations

import argparse
import json
import math
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from .core import Capture, Process, read_processes
from .detection import codex_root_bundle, matches_root, root_candidates
from .report import compare, save_report

HELP = """Commands (these mark your actions; they do not send prompts or control apps):
  idle [note]          Start an idle phase.
  work [note]          Start a work phase; use Codex in the target app yourself.
  post [note]          Start the original post-work phase.
  mark NAME [note]     Start another named phase (e.g. recovery).
  note TEXT           Record a timestamped observation.
  status              Show the last app + backend/helper RSS sample.
  footprint / snap    Capture an opt-in, intrusive group footprint diagnostic.
  done [note]         Finish with outcome measurement_only, without claiming verification.
  stop [note]         Original alias for done.
  fail REASON         Finish with outcome product_failure; keep the failed attempt.
  invalid REASON      Finish with outcome harness_invalid; keep the invalid attempt.
  help                Show these commands.
  quit                Stop with outcome incomplete.
"""


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    discovery = commands.add_parser("discover", help="List verified app roots and backend candidates")
    discovery.add_argument("--target", choices=["smaller-talk", "codex-app"])
    record = commands.add_parser("record", help="Attach to one app and all observed descendants")
    record.add_argument("--target", choices=["smaller-talk", "codex-app", "custom"], required=True)
    record.add_argument("--pid", type=int, help="Main app PID; otherwise wait for one verified GUI root")
    record.add_argument("--wait-timeout", type=float, default=60, help="Maximum seconds to wait for an app root")
    record.add_argument("--include-pid", type=int, action="append", default=[],
                        help="Explicit extra backend root; its full tree is counted")
    record.add_argument("--label", required=True)
    record.add_argument("--context", choices=["busy", "controlled", "unknown"], default="busy")
    record.add_argument("--interval", type=float, default=1.0)
    record.add_argument("--duration", type=float, help="Timed measurement without interactive commands")
    record.add_argument("--phase", default="idle", help="Initial phase name")
    record.add_argument("--workload", type=Path, help="JSON workload/configuration manifest to embed and hash")
    record.add_argument("--output", type=Path, help="New directory; existing captures are never overwritten")
    report = commands.add_parser("report", help="Regenerate JSON and Markdown from a capture")
    report.add_argument("directory", type=Path)
    comparison = commands.add_parser("compare", help="Describe two captures using full process totals")
    comparison.add_argument("left", type=Path)
    comparison.add_argument("right", type=Path)
    history = commands.add_parser("save-results", help="Save compact results from a local suite/capture into Git history")
    history.add_argument("directory", type=Path)
    history.add_argument("--results-dir", type=Path, help="Default: benchmarks/results")
    bench = commands.add_parser("bench", help="Build and run unattended Smaller Talk playback and real-Codex memory scenarios")
    bench.add_argument("--output", type=Path, help="New run directory; default is timestamped under .dev/memory-bench")
    bench.add_argument("--label", default="Smaller Talk release benchmark")
    bench.add_argument("--context", choices=["busy", "controlled", "unknown"], default="busy")
    bench.add_argument("--interval", type=float, default=1.0)
    bench.add_argument("--model", help="Real Codex model; otherwise use and record the discovered default")
    bench.add_argument("--effort", help="Real Codex reasoning effort; otherwise use the provider default")
    bench.add_argument("--duration", type=float, help="Timed collector check; does not submit the prompt")
    bench.add_argument("--manual", action="store_true", help="Original interactive phase-marker workflow")
    bench.add_argument("--offline", action="store_true", help="Playback only; explicitly excludes real Codex costs")
    bench.add_argument("--repeats", type=int, default=3, help="Independent playback launches per build (default 3; exploratory)")
    bench.add_argument("--cycles", type=int, default=3, help="Fixed-history open/close cycles per playback run")
    bench.add_argument("--settle-seconds", type=float, default=3, help="Fixed settling window; at least three sample intervals")
    bench.add_argument("--timeout", type=float, default=180, help="Maximum wait for each app operation")
    bench.add_argument("--binary", type=Path, help="Use this benchmark-capable release instead of building")
    bench.add_argument("--baseline-binary", type=Path, help="Pair with a second benchmark-capable release; randomize order")
    bench.add_argument("--codex-binary", type=Path, help="Pin the real Codex executable for live runs")
    bench.add_argument("--seed", type=int, default=1729, help="Recorded random seed for paired build order")
    return root


def discover(target: str | None = None) -> int:
    rows = read_processes()
    selected = root_candidates(target, rows) if target else sorted(set(
        root_candidates("smaller-talk", rows) + root_candidates("codex-app", rows)
    ))
    print("PID     PPID    Verified app root executable (no arguments collected)")
    for pid in selected:
        p = rows[pid]
        print(f"{p.pid:<7} {p.ppid:<7} {p.executable}")
    if not target:
        print("Backend candidates (not GUI roots):")
        for p in rows.values():
            if Path(p.executable).name.lower() == "codex" and p.pid not in selected:
                print(f"{p.pid:<7} {p.ppid:<7} {p.executable}")
    print("Select the main app PID. CLI entries can be explicit extras if not descendants.")
    print("ChatGPT.app may host Codex; its full tree also includes other ChatGPT work.")
    return 0 if not target or len(selected) == 1 else 2


def resolve_root(target: str, pid: int | None, timeout: float) -> Process:
    if pid is not None:
        rows = read_processes()
        if pid not in rows:
            raise ValueError(f"Root PID {pid} is not running")
        if target != "custom" and not matches_root(target, rows[pid]):
            raise ValueError(f"PID {pid} is not a verified {target} GUI root; use custom for collector probes")
        return rows[pid]
    if target == "custom":
        raise ValueError("Custom targets require --pid")
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Wait timeout must be finite and positive")
    print(f"Waiting up to {timeout:g}s for one {target} GUI root. Launch the app in another terminal.", flush=True)
    deadline = time.monotonic() + timeout
    while True:
        rows = read_processes()
        candidates = root_candidates(target, rows)
        if len(candidates) == 1:
            print(f"Detected GUI root PID {candidates[0]}", flush=True)
            return rows[candidates[0]]
        if len(candidates) > 1:
            raise ValueError("Multiple GUI roots found; choose --pid explicitly")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ValueError("No verified GUI root found; no memory result was recorded")
        time.sleep(min(0.25, remaining))


def dispatch(capture: Capture, command: str) -> tuple[str, str] | None:
    words = shlex.split(command)
    if not words:
        return None
    name, *args = words
    text = " ".join(args)
    if name in {"idle", "work", "post"}:
        capture.mark(name, text)
    elif name == "mark":
        if not args:
            raise ValueError("mark needs a phase name")
        capture.mark(args[0], " ".join(args[1:]))
    elif name == "note":
        if not text:
            raise ValueError("note needs text")
        capture.emit("note", text=text)
    elif name == "status":
        with capture.lock:
            sample = capture.last_sample
        if sample:
            print(json.dumps({key: sample[key] for key in (
                "elapsed_s", "phase", "root_present", "native_rss_kib",
                "descendants_rss_kib", "codex_backend_rss_kib", "total_rss_kib",
            )}, indent=2))
        else:
            print("No successful sample yet.")
    elif name in {"footprint", "snap"}:
        print(json.dumps(capture.footprint(), indent=2))
    elif name in {"help", "?"}:
        print(HELP)
    elif name in {"done", "stop"}:
        return "measurement_only", text
    elif name in {"fail", "invalid"}:
        if not text:
            raise ValueError(f"{name} needs a reason")
        return ("product_failure" if name == "fail" else "harness_invalid"), text
    elif name in {"quit", "exit"}:
        return "incomplete", text or "Operator stopped capture"
    else:
        raise ValueError(f"Unknown command {name!r}; use help")
    return None


def record(args: argparse.Namespace) -> int:
    if args.duration is not None and (not math.isfinite(args.duration) or args.duration <= 0):
        raise ValueError("Duration must be finite and positive")
    if not args.phase.strip():
        raise ValueError("Initial phase must not be empty")
    if not math.isfinite(args.interval) or args.interval < 0.05:
        raise ValueError("Sampling interval must be finite and at least 0.05 seconds")
    if not args.label.strip():
        raise ValueError("Capture label must not be empty")
    waiting_started = time.monotonic()
    root_process = resolve_root(args.target, args.pid, args.wait_timeout)
    root_pid = root_process.pid
    wait_elapsed = time.monotonic() - waiting_started
    bundle = codex_root_bundle(root_process.executable) if args.target == "codex-app" else None
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    output = args.output or Path(".dev/memory-bench") / f"{stamp}-{args.target}"
    capture = Capture(
        output, root_pid, args.include_pid, args.target, args.label,
        args.context, args.interval, args.workload,
        bundle_prefix=bundle + "/Contents/" if bundle else None,
        expected_identity=root_process.identity,
    )
    capture.manifest["root_detection_wait_s"] = wait_elapsed
    outcome, reason = "incomplete", "Capture did not finish"
    try:
        capture.mark(args.phase)
        capture.start()
        print(f"Capturing {args.target} PID {root_pid} and its process tree in {output}")
        print(f"Host context: {args.context}; workload verification: manual/unverified.")
        if args.duration is not None:
            capture.stop_event.wait(args.duration)
            outcome, reason = "measurement_only", "Timed capture; workload completion unverified"
        else:
            print(HELP)
            while True:
                try:
                    line = input("memory> " if sys.stdin.isatty() else "")
                except EOFError:
                    reason = "Input ended without a completion command"
                    break
                try:
                    decision = dispatch(capture, line)
                except (OSError, ValueError) as error:
                    print(f"Command error: {error}", file=sys.stderr)
                    capture.emit("command_error", error=str(error))
                    continue
                if decision:
                    outcome, reason = decision
                    break
    except KeyboardInterrupt:
        reason = "Interrupted by operator"
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        outcome, reason = "harness_invalid", str(error)
    finally:
        capture.finish(outcome, reason)
        report = save_report(output)
        print(f"Saved {output / 'report.md'} ({report['sample_count']} samples, {report['outcome']})")
    return report_exit_code(report)


def report_exit_code(report: dict) -> int:
    failed = report["outcome"] in {"product_failure", "harness_invalid", "incomplete"}
    return int(failed or not report["sample_count"])


def main(argv: list[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    if any(arg == "--mode" or arg.startswith("--mode=") for arg in arguments):
        return legacy_main(arguments)
    args = parser().parse_args(arguments)
    try:
        if args.command == "discover":
            return discover(args.target)
        elif args.command == "record":
            return record(args)
        elif args.command == "report":
            report = save_report(args.directory)
            print((args.directory / "report.md").read_text())
            return report_exit_code(report)
        elif args.command == "compare":
            print(compare(args.left, args.right))
        elif args.command == "save-results":
            from .history import save_results
            print(save_results(args.directory, args.results_dir))
        elif args.command == "bench":
            if args.manual or args.duration is not None:
                from .launcher import run
            else:
                from .scenarios import run
            return run(args)
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"memory-bench: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("Benchmark interrupted before capture completed.", file=sys.stderr)
        return 130


def legacy_main(argv: list[str] | None = None) -> int:
    original = argparse.ArgumentParser(description="Original v3 command-line interface over the new collector")
    original.add_argument("--mode", choices=["smaller", "codex"], required=True)
    original.add_argument("--out", type=Path)
    original.add_argument("--diagnose", action="store_true")
    original.add_argument("--pid", type=int)
    original.add_argument("--interval", type=float, default=1.0)
    original.add_argument("--duration", type=float)
    original.add_argument("--context", choices=["busy", "controlled", "unknown"], default="busy")
    original.add_argument("--label")
    original.add_argument("--workload", type=Path)
    original.add_argument("--include-pid", type=int, action="append", default=[])
    original.add_argument("--wait-timeout", type=float, default=60)
    args = original.parse_args(argv)
    target = "smaller-talk" if args.mode == "smaller" else "codex-app"
    if args.diagnose:
        return main(["discover", "--target", target])
    if args.out is None:
        original.error("benchmark runs require --out")
    command = ["record", "--target", target, "--output", str(args.out),
               "--label", args.label or args.out.name, "--interval", str(args.interval),
               "--context", args.context, "--phase", "launch", "--wait-timeout", str(args.wait_timeout)]
    for flag, value in (("--pid", args.pid), ("--duration", args.duration), ("--workload", args.workload)):
        if value is not None:
            command.extend([flag, str(value)])
    for pid in args.include_pid:
        command.extend(["--include-pid", str(pid)])
    return main(command)

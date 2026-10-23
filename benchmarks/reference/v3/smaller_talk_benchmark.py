#!/usr/bin/env python3
"""Disposable macOS process-tree benchmark for Smaller Talk vs Codex Desktop.

Version 3: identifies Codex Desktop by macOS bundle ID (`com.openai.codex`),
including releases distributed as ChatGPT.app, then measures its full process tree.
Rejects unverified Codex CLI/helper matches as GUI roots and flags empty runs.

Standard library only. Does not launch, stop, or modify the measured application.
Usage: python3 smaller_talk_benchmark.py --mode smaller --out ./runs/smaller-1
       python3 smaller_talk_benchmark.py --mode codex   --out ./runs/codex-1

Commands while running: idle, work, post, snap, stop
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import plistlib
import platform
import queue
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from functools import lru_cache
from pathlib import Path


@dataclass(frozen=True)
class Proc:
    pid: int
    ppid: int
    rss_kib: int
    cpu_pct: float
    cmd: str


def ps_snapshot() -> dict[int, Proc]:
    # -ww keeps the full command (important for Electron helpers and bundle paths).
    result = subprocess.run(
        ["ps", "-axww", "-o", "pid=,ppid=,rss=,pcpu=,command="],
        capture_output=True, text=True, check=True,
    )
    found = {}
    for line in result.stdout.splitlines():
        parts = line.split(None, 4)
        if len(parts) != 5:
            continue
        try:
            pid, ppid, rss = map(int, parts[:3])
            cpu = float(parts[3])
        except ValueError:
            continue
        found[pid] = Proc(pid, ppid, rss, cpu, parts[4])
    return found


# Codex Desktop's external name is not a reliable process identifier. In particular,
# Codex Desktop can be installed as ChatGPT.app with CFBundleIdentifier=com.openai.codex;
# the separately installed ChatGPT desktop app may have a different bundle ID.
# Never treat pgrep -x Codex results (which may all be *children*) as app roots.
CODEX_BUNDLE_ID = "com.openai.codex"
APP_MAIN_EXECUTABLE = re.compile(
    r"^(?P<bundle>/.*?\.app)/Contents/MacOS/(?P<executable>[^/\s]+)(?=\s|$)"
)


@lru_cache(maxsize=64)
def app_info(bundle_path: str) -> tuple[str | None, str | None]:
    """Read a local app's advertised bundle ID and executable; no external tools."""
    try:
        with (Path(bundle_path) / "Contents" / "Info.plist").open("rb") as handle:
            info = plistlib.load(handle)
        return info.get("CFBundleIdentifier"), info.get("CFBundleExecutable")
    except (OSError, ValueError, TypeError, plistlib.InvalidFileException):
        return None, None


def codex_root_bundle(cmd: str) -> str | None:
    match = APP_MAIN_EXECUTABLE.match(cmd)
    if match is None:
        return None
    bundle = match.group("bundle")
    identifier, executable = app_info(bundle)
    if identifier != CODEX_BUNDLE_ID or match.group("executable") != executable:
        return None
    # Electron helpers and renderers should never be treated as the main app.
    if "--type=" in cmd or " --utility-sub-type=" in cmd:
        return None
    return bundle


def matches_root(mode: str, cmd: str) -> bool:
    if mode == "smaller":
        # Only match executable argv[0], not `cargo run ...` or benchmark flags.
        return bool(re.match(r"^(?:.*?/)?smaller-talk-desktop(?=\s|$)", cmd))
    return codex_root_bundle(cmd) is not None


def root_candidates(mode: str, processes: dict[int, Proc]) -> list[int]:
    matches = {p.pid for p in processes.values() if matches_root(mode, p.cmd)}
    # If a GUI executable launches a same-name child, select the ancestor,
    # rather than reporting both as independent applications.
    roots = []
    for pid in matches:
        parent = processes[pid].ppid
        ancestors = set()
        while parent in processes and parent not in ancestors:
            ancestors.add(parent)
            if parent in matches:
                break
            parent = processes[parent].ppid
        else:
            roots.append(pid)
    return sorted(roots)


def candidate_label(p: Proc, mode: str) -> str:
    return "Codex Desktop main (bundle ID verified)" if mode == "codex" else "Smaller Talk main"


def diagnose(mode: str) -> int:
    processes = ps_snapshot()
    roots = root_candidates(mode, processes)
    print(f"Diagnostic scan for {mode}: {len(roots)} verified GUI root(s) found")
    for pid in roots:
        p = processes[pid]
        print(f"  PID {pid:>7} PPID {p.ppid:>7} — {candidate_label(p, mode)}")
        if mode == "codex":
            print(f"      App bundle: {Path(codex_root_bundle(p.cmd) or '').name}  "
                  f"ID: {CODEX_BUNDLE_ID}")
    if len(roots) == 1:
        family = app_family(mode, processes, roots[0])
        print(f"  Total detected app tree: {len(family)} process(es), including root")
    if not roots:
        print("No verified GUI root detected. Launch the app, rerun --diagnose, or")
        print("use --pid <MAIN_PID> after verifying its macOS bundle identifier.")
        print("Codex Desktop may be installed as ChatGPT.app; its bundle ID is com.openai.codex.")
        return 2
    if len(roots) > 1:
        print("Multiple verified app roots found; specify --pid <MAIN_PID>.")
        return 2
    return 0


def app_family(mode: str, processes: dict[int, Proc], root_pid: int) -> dict[int, Proc]:
    family = {root_pid}
    # Include all descendants recursively: renderer, app-server, tool/runtime helpers.
    for _ in range(len(processes) + 1):
        before = len(family)
        family.update(p.pid for p in processes.values() if p.ppid in family)
        if len(family) == before:
            break
    if mode == "codex" and root_pid in processes:
        # Include helpers detached from the process tree only if their argv[0]
        # still points into *this specific verified* app bundle. Don't match
        # generic `codex` or other ChatGPT processes system-wide.
        bundle = codex_root_bundle(processes[root_pid].cmd)
        if bundle:
            prefix = bundle + "/Contents/"
            family.update(p.pid for p in processes.values() if p.cmd.startswith(prefix))
            # Follow subprocesses spawned by those reparented helpers as well.
            for _ in range(len(processes) + 1):
                before = len(family)
                family.update(p.pid for p in processes.values() if p.ppid in family)
                if len(family) == before:
                    break
    return {pid: processes[pid] for pid in family if pid in processes}


def process_role(mode: str, proc: Proc, root_pid: int) -> str:
    if proc.pid == root_pid:
        return "ui_main"
    c = proc.cmd.lower()
    if "app-server" in c or "codex app-server" in c:
        return "agent_backend"
    if mode == "codex" and ("codex helper" in c or "chatgpt helper" in c or "--type=" in c):
        return "ui_helper"
    if "node_repl" in c or re.search(r"(?:^|/)node(?:\s|$)", c):
        return "runtime_helper"
    return "other_child"


def safe_process_name(mode: str, proc: Proc, root_pid: int) -> str:
    """Deliberately avoid writing the full command line (may contain private arguments)."""
    role = process_role(mode, proc, root_pid)
    if role == "ui_main":
        return "Smaller Talk" if mode == "smaller" else "Codex Desktop"
    if role == "agent_backend":
        return "codex app-server"
    if "codex helper" in proc.cmd.lower() or "chatgpt helper" in proc.cmd.lower():
        return "Codex UI Helper"
    if "node_repl" in proc.cmd:
        return "node_repl"
    return role


def run_capture(args: list[str], timeout_seconds: int = 30) -> tuple[str, int]:
    try:
        process = subprocess.run(args, text=True, capture_output=True, timeout=timeout_seconds)
        return (process.stdout + ("\nSTDERR:\n" + process.stderr if process.stderr else ""), process.returncode)
    except (OSError, subprocess.TimeoutExpired) as error:
        return (str(error), -1)


def sysctl_value(name: str) -> str | None:
    out, status = run_capture(["sysctl", "-n", name], 3)
    return out.strip() if status == 0 else None


def swap_used_mib() -> float | None:
    text = sysctl_value("vm.swapusage")
    if not text:
        return None
    match = re.search(r"\bused\s*=\s*([0-9.]+)\s*([KMGT])", text, re.I)
    if not match:
        return None
    factor = {"K": 1 / 1024, "M": 1, "G": 1024, "T": 1024 * 1024}
    return round(float(match.group(1)) * factor[match.group(2).upper()], 2)


def summarize(samples: list[dict]) -> dict:
    result = {}
    for phase in ["launch", "idle", "work", "post"]:
        rows = [r for r in samples if r["phase"] == phase and r["process_count"] > 0]
        if not rows:
            continue
        rss = [r["rss_total_mib"] for r in rows]
        cpu = [r["cpu_total_pct"] for r in rows]
        result[phase] = {
            "sample_count": len(rows),
            "first_elapsed_s": rows[0]["elapsed_s"],
            "last_elapsed_s": rows[-1]["elapsed_s"],
            "rss_total_mean_mib": round(statistics.mean(rss), 1),
            "rss_total_median_mib": round(statistics.median(rss), 1),
            "rss_total_peak_mib": round(max(rss), 1),
            "cpu_mean_pct": round(statistics.mean(cpu), 1),
            "cpu_peak_pct": round(max(cpu), 1),
            "process_count_peak": max(r["process_count"] for r in rows),
            "ui_rss_mean_mib": round(statistics.mean(r["rss_ui_mib"] for r in rows), 1),
            "backend_rss_mean_mib": round(statistics.mean(r["rss_backend_mib"] for r in rows), 1),
        }
    return result


def input_worker(commands: queue.Queue):
    while True:
        line = sys.stdin.readline()
        if line == "":
            return
        commands.put(line.strip().lower())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mode", choices=["smaller", "codex"], help="App to measure")
    parser.add_argument("--out", type=Path, help="New output directory for this run")
    parser.add_argument("--diagnose", action="store_true", help="Show detected GUI process roots and exit (open app first)")
    parser.add_argument("--interval", type=float, default=1.0, help="Sample interval in seconds (default 1)")
    parser.add_argument("--pid", type=int, help="Optional exact root PID if app detection fails")
    parser.add_argument("--duration", type=float, help="Optional timed, unattended run (seconds; useful for checks)")
    args = parser.parse_args()
    if args.diagnose:
        if args.mode is None:
            parser.error("--diagnose also requires --mode")
        return diagnose(args.mode)
    if args.mode is None or args.out is None:
        parser.error("benchmark runs require both --mode and --out")
    if args.interval < 0.25 or (args.duration is not None and args.duration <= 0):
        parser.error("--interval must be >= 0.25 and --duration must be > 0")
    out = args.out.expanduser()
    if out.exists():
        parser.error(f"output directory already exists: {out} (use a fresh name per run)")
    out.mkdir(parents=True)
    snapshots = out / "footprint"
    snapshots.mkdir()

    metadata = {
        "mode": args.mode, "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "hostname": platform.node(), "macos": platform.mac_ver()[0],
        "platform": platform.platform(), "machine": platform.machine(),
        "python": sys.version.split()[0], "logical_cpus": os.cpu_count(),
        "physical_ram_bytes": sysctl_value("hw.memsize"),
        "sample_interval_s": args.interval,
        "note": "No full process command lines are recorded to avoid leaking private arguments.",
    }
    (out / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf8")
    totals_fields = [
        "timestamp_utc", "elapsed_s", "phase", "process_count", "rss_total_mib",
        "rss_ui_mib", "rss_backend_mib", "rss_other_mib", "cpu_total_pct",
        "cpu_ui_pct", "cpu_backend_pct", "cpu_other_pct", "system_load_1m", "swap_used_mib",
    ]
    process_fields = ["timestamp_utc", "elapsed_s", "phase", "pid", "ppid", "role", "name", "rss_mib", "cpu_pct"]
    phase = "launch"
    root_pid = args.pid
    samples: list[dict] = []
    markers = []
    footprint_files = []
    commands: queue.Queue = queue.Queue()
    if args.duration is None:
        threading.Thread(target=input_worker, args=(commands,), daemon=True).start()
    print(f"\nMeasuring {args.mode.upper()} — output: {out.resolve()}", flush=True)
    print("Open the app AFTER starting this monitor. Use a SECOND terminal/window.")
    print("Commands here: idle (loaded and settled), work (about to send prompt),")
    print("               post (response finished), snap (manual footprint), stop (end run).")
    print("Wait ~30 seconds in the idle and post phases for a useful comparison.\n", flush=True)

    start = time.monotonic()
    next_sample = start
    last_log = start
    stopped = False
    last_procs: dict[int, Proc] = {}
    seen_root = False
    ever_detected = False
    warned_no_root = False
    swap_value = None
    try:
        with (out / "totals.csv").open("w", newline="", encoding="utf8") as totals_file, \
             (out / "processes.csv").open("w", newline="", encoding="utf8") as processes_file:
            tw = csv.DictWriter(totals_file, fieldnames=totals_fields)
            pw = csv.DictWriter(processes_file, fieldnames=process_fields)
            tw.writeheader()
            pw.writeheader()
            while True:
                while True:
                    try:
                        command = commands.get_nowait()
                    except queue.Empty:
                        break
                    if command in {"idle", "work", "post"}:
                        phase = command
                        markers.append({"phase": phase, "elapsed_s": round(time.monotonic() - start, 2)})
                        print(f"Marked {phase.upper()} at {markers[-1]['elapsed_s']}s", flush=True)
                        if command in {"idle", "post"}:
                            # Apple footprint de-duplicates shared regions across multiple PIDs.
                            if shutil.which("footprint") and last_procs:
                                pids = sorted(last_procs)
                                label = f"{len(footprint_files)+1:02d}_{command}"
                                cmd = ["footprint", "-f", "bytes"]
                                for pid in pids:
                                    cmd += ["-p", str(pid)]
                                report, exit_code = run_capture(cmd, 35)
                                file = snapshots / f"{label}.txt"
                                file.write_text(report, encoding="utf8")
                                footprint_files.append({"phase": phase, "file": str(file.relative_to(out)), "pids": pids, "exit_code": exit_code})
                                print(f"  Footprint snapshot: {file} (exit {exit_code})", flush=True)
                            else:
                                print("  No app processes yet, or macOS footprint unavailable.", flush=True)
                    elif command == "snap":
                        if not shutil.which("footprint") or not last_procs:
                            print("No detected app processes / footprint unavailable.", flush=True)
                        else:
                            pids = sorted(last_procs)
                            label = f"{len(footprint_files)+1:02d}_{phase}_manual"
                            cmd = ["footprint", "-f", "bytes"]
                            for pid in pids:
                                cmd += ["-p", str(pid)]
                            report, exit_code = run_capture(cmd, 35)
                            file = snapshots / f"{label}.txt"
                            file.write_text(report, encoding="utf8")
                            footprint_files.append({"phase": phase, "file": str(file.relative_to(out)), "pids": pids, "exit_code": exit_code})
                            print(f"  Footprint snapshot: {file} (exit {exit_code})", flush=True)
                    elif command in {"stop", "quit", "exit"}:
                        stopped = True
                    elif command == "help":
                        print("idle | work | post | snap | stop", flush=True)
                    elif command:
                        print(f"Unknown command {command!r}. Type help.", flush=True)
                if stopped or (args.duration is not None and time.monotonic() - start >= args.duration):
                    break

                now = time.monotonic()
                if now < next_sample:
                    time.sleep(min(0.15, next_sample - now))
                    continue
                elapsed = round(now - start, 2)
                next_sample = now + args.interval
                try:
                    all_procs = ps_snapshot()
                except (OSError, subprocess.CalledProcessError) as error:
                    print(f"ps failed: {error}", file=sys.stderr)
                    continue
                if root_pid is None:
                    matching = root_candidates(args.mode, all_procs)
                    if len(matching) == 1:
                        root_pid = matching[0]
                        print(f"Detected {args.mode} app main process: PID {root_pid}", flush=True)
                    elif len(matching) > 1:
                        if not warned_no_root:
                            warned_no_root = True
                            print(f"WARNING: {len(matching)} verified GUI roots; refusing to guess. Use --pid.", flush=True)
                if not ever_detected and not warned_no_root and elapsed >= 15 and root_pid is None:
                    warned_no_root = True
                    print("WARNING: No matching app root after 15 seconds. This run has no memory data yet.", flush=True)
                    print(f"         Open the app, then check: python3 smaller_talk_benchmark.py --mode {args.mode} --diagnose", flush=True)
                    print("         If necessary, rerun with --pid <MAIN_PID>.", flush=True)
                if root_pid in all_procs:
                    ever_detected = True
                    if not seen_root:
                        seen_root = True
                    group = app_family(args.mode, all_procs, root_pid)
                else:
                    group = {}
                last_procs = group
                if seen_root and not group:
                    print("App main process exited. Type stop, or relaunch in a new monitor run.", flush=True)
                    seen_root = False
                    root_pid = None if args.pid is None else args.pid
                if int(elapsed / args.interval) % max(1, round(10 / args.interval)) == 0:
                    swap_value = swap_used_mib()
                timestamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
                roles = {pid: process_role(args.mode, p, root_pid if root_pid is not None else -1) for pid, p in group.items()}
                rss_ui = sum(p.rss_kib for p in group.values() if roles[p.pid] in {"ui_main", "ui_helper"}) / 1024
                rss_backend = sum(p.rss_kib for p in group.values() if roles[p.pid] == "agent_backend") / 1024
                rss_total = sum(p.rss_kib for p in group.values()) / 1024
                cpu_ui = sum(p.cpu_pct for p in group.values() if roles[p.pid] in {"ui_main", "ui_helper"})
                cpu_backend = sum(p.cpu_pct for p in group.values() if roles[p.pid] == "agent_backend")
                cpu_total = sum(p.cpu_pct for p in group.values())
                row = {
                    "timestamp_utc": timestamp, "elapsed_s": elapsed, "phase": phase,
                    "process_count": len(group), "rss_total_mib": round(rss_total, 2),
                    "rss_ui_mib": round(rss_ui, 2), "rss_backend_mib": round(rss_backend, 2),
                    "rss_other_mib": round(rss_total - rss_ui - rss_backend, 2),
                    "cpu_total_pct": round(cpu_total, 2), "cpu_ui_pct": round(cpu_ui, 2),
                    "cpu_backend_pct": round(cpu_backend, 2),
                    "cpu_other_pct": round(cpu_total - cpu_ui - cpu_backend, 2),
                    "system_load_1m": round(os.getloadavg()[0], 2), "swap_used_mib": swap_value,
                }
                tw.writerow(row)
                for pid, proc in group.items():
                    pw.writerow({
                        "timestamp_utc": timestamp, "elapsed_s": elapsed, "phase": phase,
                        "pid": pid, "ppid": proc.ppid, "role": roles[pid],
                        "name": safe_process_name(args.mode, proc, root_pid if root_pid is not None else -1),
                        "rss_mib": round(proc.rss_kib / 1024, 2), "cpu_pct": round(proc.cpu_pct, 2),
                    })
                samples.append(row)
                totals_file.flush()
                processes_file.flush()
                if now - last_log >= 5:
                    print(f"[{phase:<6}] {len(group):>2} processes  {rss_total:>7.1f} MiB summed RSS  {cpu_total:>5.1f}% CPU", flush=True)
                    last_log = now
    except KeyboardInterrupt:
        print("\nInterrupted; saving what was collected.", flush=True)

    summary = {
        "mode": args.mode,
        "valid_process_capture": ever_detected,
        "total_samples": len(samples),
        "elapsed_seconds": round(time.monotonic() - start, 1),
        "root_pid_last": root_pid,
        "phase_statistics": summarize(samples),
        "markers": markers,
        "footprint_snapshots": footprint_files,
        "cautions": [
            "Summed RSS double-counts shared physical pages across processes; inspect multi-PID footprint reports for de-duplicated macOS memory accounting.",
            "ps CPU% is an approximate moving average per process; sums may exceed 100% on multicore Macs.",
            "Backend, UI helper and other process attribution is heuristic; verify processes.csv before relying on split totals.",
            "System load and swap are system-wide, not costs attributable to the measured app.",
            "Tool calls, network/model variability, idle elapsed time and other apps affect comparability.",
            "GPU memory, energy/watts and attributable disk/network I/O are not recorded.",
        ],
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf8")
    print(f"\nSaved {len(samples)} samples and summary in {out.resolve()}")
    if not ever_detected:
        print("INVALID RUN: No measured application process was ever detected; do not compare zero-RSS data.", file=sys.stderr)
        print("Open the application and run --diagnose to identify its process, then repeat the benchmark.", file=sys.stderr)
        return 2
    for phase_name, s in summary["phase_statistics"].items():
        print(f"  {phase_name:<6} median {s['rss_total_median_mib']:>7.1f} MiB RSS  peak {s['rss_total_peak_mib']:>7.1f}  mean CPU {s['cpu_mean_pct']:>5.1f}%  n={s['sample_count']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

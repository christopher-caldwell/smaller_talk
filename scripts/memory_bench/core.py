"""Process accounting and append-only capture; no UI or provider control."""

from __future__ import annotations

import hashlib
import json
import math
import os
import platform
import plistlib
import subprocess
import threading
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

SCHEMA_VERSION = 1
LIMITATIONS = [
    "RSS sums can double-count shared pages and exclude compressed/swapped pages.",
    "Sampled maxima are not true allocation peaks; short-lived children can be missed.",
    "Process identity uses PID and ps start time, whose resolution is one second.",
    "Reparented children are retained only if previously observed in the target tree.",
    "External/shared backends require explicit inclusion; their work is not apportioned.",
    "Backend labels are executable-name heuristics; total includes all observed descendants.",
    "The collector and observed helper children are excluded; shared parent tool hosts are not.",
    "ps CPU percentages are moving estimates, not interval CPU time; totals can exceed 100%.",
    "Manual capture does not establish equivalent work, UI completion, or a performance verdict.",
]


@dataclass(frozen=True)
class Process:
    pid: int
    ppid: int
    started: str
    rss_kib: int
    executable: str
    cpu_pct: float = 0.0

    @property
    def identity(self) -> tuple[int, str]:
        return self.pid, self.started


def parse_ps(output: str) -> dict[int, Process]:
    """Parse LC_ALL=C ps with executable paths, never command arguments."""
    rows = {}
    for line in output.splitlines():
        if not line.strip():
            continue
        fields = line.split(None, 9)
        if len(fields) != 10:
            raise ValueError(f"Unexpected ps row shape ({len(fields)} fields)")
        pid, ppid = int(fields[0]), int(fields[1])
        started = " ".join(fields[2:7])
        datetime.strptime(started, "%a %b %d %H:%M:%S %Y")
        rss = int(fields[7])
        cpu = float(fields[8])
        if pid <= 0 or ppid < 0 or rss < 0 or pid in rows or not math.isfinite(cpu) or cpu < 0:
            raise ValueError("Invalid or duplicate process in ps snapshot")
        rows[pid] = Process(pid, ppid, started, rss, fields[9], cpu)
    if not rows:
        raise ValueError("Empty process snapshot")
    return rows


def read_processes() -> dict[int, Process]:
    if platform.system() != "Darwin":
        raise RuntimeError("Live collection currently requires macOS")
    result = subprocess.run(
        ["/bin/ps", "-ww", "-axo", "pid=,ppid=,lstart=,rss=,pcpu=,comm="],
        env={**os.environ, "LC_ALL": "C", "TZ": "UTC"},
        capture_output=True,
        text=True,
        check=True,
        timeout=5,
    )
    return parse_ps(result.stdout)


def backend_kind(executable: str) -> str | None:
    name = Path(executable).name.lower()
    if name in {"codex", "codex-code-mode-host", "codex-app-server"}:
        return "codex"
    if name == "claude":
        return "claude"
    return None


def process_role(process: Process, root_identity: tuple[int, str], bundle_prefix: str | None) -> str:
    if process.identity == root_identity:
        return "ui_main"
    if backend_kind(process.executable):
        return "agent_backend"
    if bundle_prefix and process.executable.startswith(bundle_prefix) and "/Contents/Frameworks/" in process.executable:
        return "ui_helper"
    if Path(process.executable).name in {"node", "node_repl"}:
        return "runtime_helper"
    return "other_child"


class ProcessTree:
    """Pin identities and keep observed descendants after they are reparented."""

    def __init__(self, root: Process, extras: list[Process], observer: Process | None = None,
                 bundle_prefix: str | None = None):
        self.root = root.identity
        self.seeds = {root.identity, *(p.identity for p in extras)}
        self.owned = set(self.seeds)
        self.observer_owned = {observer.identity} if observer else set()
        self.bundle_prefix = bundle_prefix
        self.previous: set[tuple[int, str]] = set()

    @staticmethod
    def descendants(rows, seeds, protected=frozenset()):
        selected = {p.identity for p in rows.values() if p.identity in seeds}
        changed = True
        while changed:
            changed = False
            for p in rows.values():
                parent = rows.get(p.ppid)
                if parent and parent.identity in selected and p.identity not in selected and p.identity not in protected:
                    selected.add(p.identity)
                    changed = True
        return selected

    def select(self, rows: dict[int, Process]) -> set[tuple[int, str]]:
        excluded = self.descendants(rows, self.observer_owned, self.seeds)
        bundle_members = set()
        root_present = any(p.identity == self.root for p in rows.values())
        if self.bundle_prefix and root_present:
            bundle_members = {p.identity for p in rows.values()
                              if p.executable.startswith(self.bundle_prefix)}
        return self.descendants(rows, self.owned | bundle_members) - excluded

    def observe(self, rows: dict[int, Process]) -> dict:
        self.observer_owned.update(self.descendants(rows, self.observer_owned, self.seeds))
        selected = self.select(rows)
        self.owned.update(selected)
        processes = sorted(
            (p for p in rows.values() if p.identity in selected), key=lambda p: p.pid
        )
        native = sum(p.rss_kib for p in processes if p.identity == self.root)
        descendants = sum(p.rss_kib for p in processes if p.identity != self.root)
        codex = sum(
            p.rss_kib
            for p in processes
            if p.identity != self.root and backend_kind(p.executable) == "codex"
        )
        new = selected - self.previous
        exited = self.previous - selected
        self.previous = selected
        current_identities = {p.identity for p in rows.values()}
        return {
            "root_present": self.root in selected,
            "observer_excluded": [list(identity) for identity in sorted(self.observer_owned)
                                  if identity in current_identities],
            "native_rss_kib": native if self.root in selected else None,
            "descendants_rss_kib": descendants,
            "codex_backend_rss_kib": codex,
            "total_rss_kib": native + descendants,
            "total_cpu_pct": sum(p.cpu_pct for p in processes),
            "native_cpu_pct": sum(p.cpu_pct for p in processes if p.identity == self.root)
                              if self.root in selected else None,
            "codex_backend_cpu_pct": sum(p.cpu_pct for p in processes if p.identity != self.root
                                         and backend_kind(p.executable) == "codex"),
            "processes": [
                {
                    **asdict(p),
                    "role": process_role(p, self.root, self.bundle_prefix),
                    "backend": backend_kind(p.executable),
                    "explicit_root": p.identity in self.seeds,
                    "bundle_path_match": bool(self.bundle_prefix and p.executable.startswith(self.bundle_prefix)),
                }
                for p in processes
            ],
            "appeared": [list(identity) for identity in sorted(new)],
            "disappeared": [list(identity) for identity in sorted(exited)],
        }


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def executable_metadata(process: Process) -> dict:
    result = asdict(process)
    try:
        result["sha256"] = file_sha256(Path(process.executable))
    except OSError as error:
        result["hash_error"] = str(error)
    marker = ".app/Contents/"
    if marker in process.executable:
        bundle = Path(process.executable.split(marker, 1)[0] + ".app")
        try:
            with (bundle / "Contents/Info.plist").open("rb") as source:
                info = plistlib.load(source)
            if not isinstance(info, dict):
                raise ValueError("App Info.plist must be a dictionary")
            result["bundle"] = {
                "path": str(bundle),
                **{
                    key: info.get(key)
                    for key in (
                        "CFBundleIdentifier",
                        "CFBundleShortVersionString",
                        "CFBundleVersion",
                    )
                },
            }
        except (OSError, ValueError, plistlib.InvalidFileException) as error:
            result["bundle_error"] = str(error)
    return result


def command_metadata(command: list[str], cwd: Path | None = None) -> dict:
    try:
        result = subprocess.run(
            command, cwd=cwd, capture_output=True, text=True, timeout=5, check=True
        )
        return {"value": result.stdout.strip()}
    except (OSError, subprocess.SubprocessError) as error:
        return {"unavailable": str(error)}


def host_metadata() -> dict:
    return {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "macos": platform.mac_ver()[0],
        "python": platform.python_version(),
        "cpu_count": os.cpu_count(),
        "memory_bytes": command_metadata(["/usr/sbin/sysctl", "-n", "hw.memsize"]),
    }


def host_memory() -> dict:
    return {
        "vm_stat": command_metadata(["/usr/bin/vm_stat"]),
        "swap": command_metadata(["/usr/sbin/sysctl", "vm.swapusage"]),
    }


def write_json(path: Path, data: dict) -> None:
    staged = path.with_suffix(path.suffix + ".new")
    staged.write_text(json.dumps(data, indent=2) + "\n")
    staged.replace(path)


class Capture:
    def __init__(
        self,
        output: Path,
        root_pid: int,
        extra_pids: list[int],
        target: str,
        label: str,
        context: str,
        interval: float,
        workload: Path | None = None,
        reader: Callable[[], dict[int, Process]] = read_processes,
        bundle_prefix: str | None = None,
        expected_identity: tuple[int, str] | None = None,
        footprint_reader: Callable[[int], dict] | None = None,
    ):
        if not math.isfinite(interval) or interval < 0.05:
            raise ValueError("Sampling interval must be finite and at least 0.05 seconds")
        if not label.strip():
            raise ValueError("Capture label must not be empty")
        rows = reader()
        if root_pid not in rows:
            raise ValueError(f"Root PID {root_pid} is not running")
        if expected_identity is not None and rows[root_pid].identity != expected_identity:
            raise ValueError("Root process identity changed before attachment")
        if os.getpid() in {root_pid, *extra_pids}:
            raise ValueError("The collector cannot be a target or extra root")
        if len(set(extra_pids)) != len(extra_pids) or root_pid in extra_pids:
            raise ValueError("Root and extra PIDs must be distinct")
        missing = set(extra_pids) - rows.keys()
        if missing:
            raise ValueError(f"Extra PIDs are not running: {sorted(missing)}")
        workload_data = None
        if workload:
            workload_bytes = workload.read_bytes()
            workload_data = {
                "path": str(workload.resolve()),
                "sha256": hashlib.sha256(workload_bytes).hexdigest(),
                "content": json.loads(workload_bytes),
            }
        output.mkdir(parents=True, exist_ok=False, mode=0o700)
        self.output = output
        self.reader = reader
        self.footprint_reader = footprint_reader
        self.rusage_identities: dict[tuple[int, str], int] = {}
        self.interval = interval
        observer = rows.get(os.getpid())
        self.tree = ProcessTree(rows[root_pid], [rows[pid] for pid in extra_pids], observer, bundle_prefix)
        self.lock = threading.RLock()
        self.stop_event = threading.Event()
        self.thread: threading.Thread | None = None
        self.phase_id = 0
        self.phase = "unlabeled"
        self.finished = False
        self.assessed = False
        self.collector_failure: str | None = None
        self.last_sample: dict | None = None
        self.origin = time.monotonic()
        self.stream = (output / "events.jsonl").open("x", buffering=1)
        repo = Path(__file__).resolve().parents[2]
        self.manifest = {
            "schema_version": SCHEMA_VERSION,
            "started_at": utc_now(),
            "target": target,
            "label": label,
            "context": context,
            "interval_s": interval,
            "footprint_accounting": ("libproc proc_pid_rusage RUSAGE_INFO_V0; sum of per-process accounting, "
                                     "not deduplicated group physical memory") if footprint_reader else None,
            "scope": {
                "root": executable_metadata(rows[root_pid]),
                "extra_roots": [executable_metadata(rows[pid]) for pid in extra_pids],
                "observer_exclusion": asdict(observer) if observer else None,
                "codex_bundle_prefix": bundle_prefix,
            },
            "host": host_metadata(),
            "host_memory_start": host_memory(),
            "framework_revision": command_metadata(["git", "rev-parse", "HEAD"], repo),
            "framework_status": command_metadata(["git", "status", "--porcelain"], repo),
            "framework_files_sha256": {
                str(path.relative_to(repo)): file_sha256(path)
                for path in [*sorted((repo / "scripts").glob("*.py")), *sorted(Path(__file__).parent.glob("*.py"))]
            },
            "workload": workload_data,
            "workload_verification": "not_verified_manual_capture",
            "performance_verdict": "not_assessed",
            "outcome": "incomplete",
            "limitations": LIMITATIONS,
        }
        self.origin = time.monotonic()
        self.manifest["started_at"] = utc_now()
        write_json(output / "manifest.json", self.manifest)
        self.emit("started", root_pid=root_pid, extra_pids=extra_pids)

    def emit(self, kind: str, **data) -> dict:
        with self.lock:
            event = {
                "type": kind,
                "elapsed_s": time.monotonic() - self.origin,
                "at": utc_now(),
                **data,
            }
            self.stream.write(json.dumps(event) + "\n")
            return event

    def mark(self, name: str, note: str = "") -> None:
        if not name.strip():
            raise ValueError("Phase name must not be empty")
        with self.lock:
            self.phase_id += 1
            self.phase = name
            self.emit("phase", phase_id=self.phase_id, phase=name, note=note)

    def sample(self) -> None:
        started = time.monotonic()
        with self.lock:
            phase_id, phase = self.phase_id, self.phase
        try:
            rows = self.reader()
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            self.emit("sample_error", error=str(error))
            return
        with self.lock:
            observation = self.tree.observe(rows)
            if self.footprint_reader:
                for process in observation["processes"]:
                    try:
                        usage = self.footprint_reader(process["pid"])
                        identity = (process["pid"], process["started"])
                        start = usage["rusage_start_abstime"]
                        if self.rusage_identities.setdefault(identity, start) != start:
                            raise ValueError("Process identity changed during footprint accounting")
                        process.update(usage)
                    except (OSError, ValueError) as error:
                        process["footprint_error"] = str(error)
                complete = observation["root_present"] and all("footprint_kib" in p for p in observation["processes"])
                observation["total_footprint_kib"] = sum(p["footprint_kib"] for p in observation["processes"]) if complete else None
                observation["native_footprint_kib"] = next((p.get("footprint_kib") for p in observation["processes"] if p["pid"] == self.tree.root[0]), None)
            event = self.emit(
                "sample",
                collection_started_s=started - self.origin,
                collection_duration_s=time.monotonic() - started,
                phase_id=phase_id,
                phase=phase,
                phase_boundary=phase_id != self.phase_id,
                host_load_1m=os.getloadavg()[0],
                **observation,
            )
            self.last_sample = event

    def start(self) -> None:
        def collect() -> None:
            deadline = time.monotonic() + self.interval
            try:
                while not self.stop_event.wait(max(0, deadline - time.monotonic())):
                    self.sample()
                    deadline += self.interval
                    now = time.monotonic()
                    if deadline < now:
                        deadline = now + self.interval
            except Exception as error:
                self.collector_failure = f"{type(error).__name__}: {error}"
                self.stop_event.set()

        if self.thread is not None:
            raise RuntimeError("Capture is already running")
        self.sample()
        self.thread = threading.Thread(target=collect, name="memory-sampler", daemon=True)
        self.thread.start()

    def footprint(self) -> dict:
        """Opt-in diagnostic; its duration remains visible in the event log."""
        rows = self.reader()
        with self.lock:
            sample = self.last_sample
            if not sample:
                raise ValueError("No sample available yet")
            members = [Process(**{k: p[k] for k in Process.__dataclass_fields__})
                       for p in sample["processes"]]
        if not members:
            raise ValueError("No observed target processes remain")
        if any(rows.get(p.pid, None) is None or rows[p.pid].identity != p.identity
               for p in members):
            raise ValueError("Process membership changed; retry after the next sample")
        with self.lock:
            if self.tree.select(rows) != {p.identity for p in members}:
                raise ValueError("Process membership changed; retry after the next sample")
        token = f"footprint-{time.monotonic_ns()}"
        destination = self.output / f"{token}.json"
        command = ["/usr/bin/footprint", "-f", "bytes", "-j", str(destination)]
        for p in members:
            command.extend(["-p", str(p.pid)])
        self.emit("diagnostic_start", diagnostic="footprint", token=token,
                  identities=[list(p.identity) for p in members])
        result_data = {"token": token, "diagnostic": "footprint"}
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=30)
            (self.output / f"{token}.txt").write_text(result.stdout + result.stderr)
            after = self.reader()
            with self.lock:
                changed = self.tree.select(after) != {p.identity for p in members}
            try:
                footprint_data = json.loads(destination.read_text())
                valid_json = (isinstance(footprint_data, dict) and not footprint_data.get("errors")
                              and isinstance(footprint_data.get("processes"), list)
                              and {p.pid for p in members} == {
                                  p["pid"] for p in footprint_data.get("processes", [])
                                  if isinstance(p, dict) and isinstance(p.get("pid"), int)
                              })
            except (OSError, ValueError):
                valid_json = False
            result_data.update(returncode=result.returncode, membership_changed=changed,
                               usable=result.returncode == 0 and not changed
                               and valid_json)
        except (OSError, subprocess.SubprocessError) as error:
            result_data.update(usable=False, error=str(error))
        self.emit("diagnostic_end", **result_data)
        return result_data

    def finish(self, outcome: str = "measurement_only", reason: str = "") -> None:
        if self.finished:
            return
        self.stop_event.set()
        if self.thread:
            self.thread.join()
        if self.collector_failure and outcome in {"measurement_only", "completed"}:
            outcome, reason = "harness_invalid", self.collector_failure
        self.emit("finished", outcome=outcome, reason=reason)
        self.manifest.update(
            finished_at=utc_now(), outcome=outcome, reason=reason,
            collector_failure=self.collector_failure,
            host_memory_end=host_memory(),
        )
        write_json(self.output / "manifest.json", self.manifest)
        self.stream.close()
        self.finished = True

    def assess(self, outcome: str, reason: str) -> None:
        """Append a final assessment after sampling stops and app cleanup is checked."""
        with self.lock:
            if not self.finished or self.assessed:
                raise RuntimeError("Final assessment requires one finished, unassessed capture")
            event = {"type": "assessment", "elapsed_s": time.monotonic() - self.origin,
                     "at": utc_now(), "outcome": outcome, "reason": reason}
            with (self.output / "events.jsonl").open("a") as stream:
                stream.write(json.dumps(event) + "\n")
            self.manifest.update(outcome=outcome, reason=reason, assessment_at=event["at"])
            write_json(self.output / "manifest.json", self.manifest)
            self.assessed = True

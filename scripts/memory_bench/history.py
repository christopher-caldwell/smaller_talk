"""Small, immutable Git-ready results; raw evidence and reports stay local."""
from __future__ import annotations

import hashlib
import json
import os
import re
import statistics
import tempfile
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

from .report import summarize

SCHEMA_VERSION = 1


def value(metadata: dict | None):
    return metadata.get("value") if isinstance(metadata, dict) else None


def load_capture(directory: Path) -> dict | None:
    if not (directory / "manifest.json").exists():
        return None
    manifest = json.loads((directory / "manifest.json").read_text())
    report = summarize(directory)
    counts = defaultdict(Counter)
    names = defaultdict(set)
    with (directory / "events.jsonl").open() as stream:
        for line in stream:
            try:
                sample = json.loads(line)
            except ValueError:
                continue
            if (not isinstance(sample, dict) or sample.get("type") != "sample"
                    or not sample["root_present"] or sample["phase_boundary"]):
                continue
            key = (sample["phase_id"], sample["phase"])
            others = [p for p in sample["processes"] if p["role"] != "ui_main"]
            counts[key]["non_app_process_samples"] += bool(others)
            counts[key]["codex_samples"] += any(p.get("backend") == "codex" for p in others)
            names[key].update(Path(p["executable"]).name.strip("()") for p in others)
    phases = []
    for phase in report["phases"]:
        metrics = {}
        for key, output, field in (
            ("native_rss_kib", "app_rss_median_mib", "median"),
            ("native_rss_kib", "app_rss_peak_mib", "sampled_max"),
            ("total_rss_kib", "group_rss_peak_mib", "sampled_max"),
            ("native_footprint_kib", "app_footprint_median_mib", "median"),
            ("native_footprint_kib", "app_footprint_peak_mib", "sampled_max"),
            ("total_footprint_kib", "group_footprint_peak_mib", "sampled_max"),
        ):
            distribution = (phase.get("footprint_metrics_kib", {}) if "footprint" in key else phase["metrics_kib"]).get(key)
            metrics[output] = distribution[field] / 1024 if distribution else None
        key = (phase["id"], phase["name"])
        phases.append({"phase": phase["name"], "sequence": phase["id"], "samples": phase["metric_samples"],
                       "non_app_process_samples": counts[key]["non_app_process_samples"],
                       "codex_samples": counts[key]["codex_samples"], "other_process_names": sorted(names[key]),
                       **metrics})
    return {"manifest": manifest, "phases": phases, "sample_errors": report["sample_errors"],
            "malformed_records": report["malformed_records"]}


def compact_results(directory: Path) -> dict:
    directory = directory.resolve()
    identity_directory = directory
    if (directory / "suite.json").exists():
        suite = json.loads((directory / "suite.json").read_text())
        trials = suite.get("trials", [])
        captures = [(trial, load_capture(Path(trial["directory"]) / "capture")) for trial in trials]
        source = {"revision": value(suite.get("source_revision")), "branch": value(suite.get("source_branch")),
                  "dirty": bool(value(suite.get("source_status"))) if value(suite.get("source_status")) is not None else None}
    else:
        capture_directory = directory if (directory / "manifest.json").exists() else directory / "capture"
        if capture_directory.name == "capture":
            identity_directory = capture_directory.parent
        capture = load_capture(capture_directory)
        if not capture:
            raise ValueError("Expected a suite.json or capture manifest")
        manifest = capture["manifest"]
        if manifest["target"] != "smaller-talk":
            raise ValueError("Results history is for Smaller Talk captures")
        suite = {"outcome": manifest["outcome"], "context": manifest["context"], "started_at": manifest["started_at"],
                 "label": manifest["label"], "interval_s": manifest["interval_s"]}
        provenance_file = capture_directory.parent / "run.json"
        provenance = json.loads(provenance_file.read_text()) if provenance_file.exists() else {}
        trial = {"kind": "manual", "build": "candidate", "outcome": manifest["outcome"],
                 "binary_sha256": manifest["scope"]["root"].get("sha256"),
                 "completion": {"settings": {key: provenance.get(key) for key in ("model", "effort")}},
                 "provider_version": provenance.get("provider_version")}
        captures = [(trial, capture)]
        # A collector revision does not establish which source built an externally attached app.
        source = {"revision": provenance.get("source_revision"), "branch": None, "dirty": None}

    first = next((capture["manifest"] for _, capture in captures if capture), {})
    match = re.match(r"(\d{8}T\d{6}(?:\.\d+)?Z)", identity_directory.name)
    inferred = None
    if match:
        stamp = match[1]
        inferred = datetime.strptime(stamp, "%Y%m%dT%H%M%S.%fZ" if "." in stamp else "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc).isoformat()
    started = suite.get("started_at") or inferred or first.get("started_at")
    if not started:
        raise ValueError("Run has no stable start timestamp; results were not saved")
    run_id = suite.get("run_id") or hashlib.sha256(f"{identity_directory.name}|{started}".encode()).hexdigest()[:16]
    host = first.get("host", {})
    groups = defaultdict(list)
    for trial, capture in captures:
        groups[(trial["kind"], trial["build"])].append((trial, capture))
    results = []
    for (kind, build), attempts in sorted(groups.items()):
        first_trial, first_capture = attempts[0]
        workload = ((first_capture or {}).get("manifest", {}).get("workload") or {}).get("content") or {}
        playback = {}
        if first_trial.get("directory"):
            config = Path(first_trial["directory"]) / "playback.json"
            if config.exists():
                playback = json.loads(config.read_text())
        actual = next((trial.get("completion", {}).get("settings") for trial, _ in attempts
                       if trial.get("completion", {}).get("settings")), {})
        by_phase = defaultdict(list)
        for trial, capture in attempts:
            if capture and trial["outcome"] in {"completed", "measurement_only"}:
                for phase in capture["phases"]:
                    by_phase[(phase["sequence"], phase["phase"])].append(phase)
        phases = []
        for (sequence, name), values in sorted(by_phase.items()):
            row = {"phase": name, "runs": len(values), "samples": sum(v["samples"] for v in values),
                   "codex_samples": sum(v["codex_samples"] for v in values),
                   "non_app_process_samples": sum(v["non_app_process_samples"] for v in values),
                   "other_process_names": sorted(set().union(*(set(v["other_process_names"]) for v in values)))}
            for key in values[0]:
                if key.endswith("_mib"):
                    available = [v[key] for v in values if v[key] is not None]
                    row[key] = round(statistics.median(available), 3) if len(available) == len(values) else None
            phases.append(row)
        results.append({"scenario": kind, "build": build, "binary_sha256": first_trial.get("binary_sha256") or (suite.get("binaries", {}).get(build) or {}).get("sha256"),
                        "attempts": dict(sorted(Counter(t["outcome"] for t, _ in attempts).items())),
                        "verification": sorted({c["manifest"].get("workload_verification", "unknown") for _, c in attempts if c}),
                        "model": actual.get("model", workload.get("model")), "effort": actual.get("effort", workload.get("effort")),
                        "provider_version": value(workload.get("provider_version")) or first_trial.get("provider_version"),
                        "provider_sha256": workload.get("provider_sha256"),
                        "workload": {"prompt_sha256": workload.get("prompt_sha256", suite.get("prompt_sha256")),
                                     "response_fixture_sha256": workload.get("expected_response_sha256"),
                                     "history_sha256": (workload.get("history") or {}).get("sha256"),
                                     "history_messages": (workload.get("history") or {}).get("messages"),
                                     "workspace_revision": workload.get("workspace_revision"),
                                     "settle_s": workload.get("settle_s"), "retention_cycles": workload.get("retention_cycles"),
                                     "chunk_chars": playback.get("chunk_chars"), "chunk_interval_s": playback.get("chunk_interval_s")},
                        "sample_errors": sum(c["sample_errors"] for _, c in attempts if c),
                        "malformed_records": sum(c["malformed_records"] for _, c in attempts if c),
                        "phases": phases})
    memory_bytes = value(host.get("memory_bytes"))
    return {"schema_version": SCHEMA_VERSION, "run_id": run_id, "started_at": started,
            "benchmark_protocol": suite.get("protocol"),
            "label": suite.get("label", first.get("label")), "context": suite.get("context", "unknown"),
            "outcome": suite["outcome"], "expected_attempts": suite.get("expected_attempts", len(captures)),
            "source": source, "host": {**{key: host.get(key) for key in ("macos", "machine", "cpu_count")},
                                       "memory_bytes": int(memory_bytes) if memory_bytes and memory_bytes.isdigit() else None},
            "interval_s": suite.get("interval_s"), "aggregation": "median of independent run summaries; peaks are sampled",
            "footprint_accounting": "sum of per-process accounting, not deduplicated group physical memory",
            "results": results}


def save_results(directory: Path, results_directory: Path | None = None) -> Path:
    record = compact_results(directory)
    if not isinstance(record["run_id"], str) or not record["run_id"]:
        raise ValueError("Run ID must be a nonempty string")
    root = results_directory or Path(__file__).resolve().parents[2] / "benchmarks/results"
    root.mkdir(parents=True, exist_ok=True)
    started = datetime.fromisoformat(record["started_at"].replace("Z", "+00:00")).astimezone(timezone.utc)
    identifier = hashlib.sha256(record["run_id"].encode()).hexdigest()[:12]
    filename = started.strftime("%Y%m%dT%H%M%S.%fZ") + "-" + identifier + ".json"
    destination = root / filename
    payload = json.dumps(record, indent=2, allow_nan=False) + "\n"
    staged = None
    try:
        with tempfile.NamedTemporaryFile("w", dir=root, prefix=".result-", suffix=".new", delete=False) as stream:
            staged = Path(stream.name)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(staged, destination)
        except FileExistsError:
            if destination.read_text() != payload:
                raise FileExistsError(f"Results already exist with different contents: {destination}")
    finally:
        if staged:
            staged.unlink(missing_ok=True)
    return destination

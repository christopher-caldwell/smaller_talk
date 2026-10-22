"""Describe captures without turning manual observations into benchmark verdicts."""

from __future__ import annotations

import json
import csv
import statistics
from collections import defaultdict
from pathlib import Path

from .core import SCHEMA_VERSION, write_json

METRICS = (
    "native_rss_kib",
    "descendants_rss_kib",
    "codex_backend_rss_kib",
    "total_rss_kib",
)
FOOTPRINT_METRICS = ("native_footprint_kib", "total_footprint_kib")


def distribution(values: list[float]) -> dict | None:
    if not values:
        return None
    return {"min": min(values), "median": statistics.median(values), "sampled_max": max(values)}


def summarize(directory: Path) -> dict:
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("Unsupported manifest schema version")
    events = []
    malformed = 0
    with (directory / "events.jsonl").open() as stream:
        for line in stream:
            try:
                event = json.loads(line)
                if not isinstance(event, dict) or "type" not in event:
                    raise ValueError("Invalid event")
                events.append(event)
            except (ValueError, TypeError):
                malformed += 1
    samples = [event for event in events if event["type"] == "sample"]
    sampling_ended_s = next((event["elapsed_s"] for event in events if event["type"] == "finished"),
                            events[-1]["elapsed_s"] if events else 0)
    grouped = defaultdict(list)
    markers = {}
    for event in events:
        if event["type"] == "phase":
            key = (event["phase_id"], event["phase"])
            grouped[key] = []
            markers[key] = event["elapsed_s"]
    for sample in samples:
        grouped[(sample["phase_id"], sample["phase"])].append(sample)
    phases = []
    ordered = sorted(grouped.items())
    for index, ((phase_id, name), data) in enumerate(ordered):
        usable = [s for s in data if s["root_present"] and not s["phase_boundary"]]
        codex_present = [s for s in usable if any(p.get("backend") == "codex" for p in s["processes"])]
        started = markers.get((phase_id, name), 0)
        ended = markers.get(ordered[index + 1][0], started) if index + 1 < len(ordered) else sampling_ended_s
        phases.append({
            "id": phase_id,
            "name": name,
            "marker_duration_s": max(0, ended - started),
            "samples": len(data),
            "metric_samples": len(usable),
            "root_missing_samples": sum(not s["root_present"] for s in data),
            "boundary_samples": sum(s["phase_boundary"] for s in data),
            "metrics_kib": {
                metric: distribution([s[metric] for s in usable]) for metric in METRICS
            },
            "footprint_metrics_kib": {
                metric: distribution([s[metric] for s in usable if s.get(metric) is not None])
                for metric in FOOTPRINT_METRICS
            },
            "footprint_samples": {metric: sum(s.get(metric) is not None for s in usable) for metric in FOOTPRINT_METRICS},
            "approx_cpu_pct": distribution([s["total_cpu_pct"] for s in usable if "total_cpu_pct" in s]),
            "codex_present": {
                "samples": len(codex_present),
                "metrics_kib": {
                    metric: distribution([s[metric] for s in codex_present]) for metric in METRICS
                },
            },
        })
    gaps = [b["collection_started_s"] - a["collection_started_s"]
            for a, b in zip(samples, samples[1:])]
    warnings = []
    if manifest["context"] != "controlled":
        warnings.append("Busy/unknown host context: these are observational captures.")
    if manifest["outcome"] == "incomplete":
        warnings.append("Capture did not finish; preserve it as an incomplete attempt.")
    if manifest.get("collector_failure"):
        warnings.append(f"Sampler stopped unexpectedly: {manifest['collector_failure']}")
    if any(not s["root_present"] for s in samples):
        warnings.append("Pinned root disappeared. Root-absent samples are retained in the trace "
                        "and excluded from memory distributions; determine normal exit vs failure.")
    errors = sum(e["type"] == "sample_error" for e in events)
    if errors or malformed:
        warnings.append(f"Collection has {errors} sample errors and {malformed} malformed records.")
    if manifest.get("footprint_accounting"):
        missing_footprint = sum(s["root_present"] and s.get("total_footprint_kib") is None for s in samples)
        if missing_footprint:
            warnings.append(f"{missing_footprint} samples have incomplete footprint accounting; totals are unavailable, not zero.")
    if any(gap > manifest["interval_s"] * 2 for gap in gaps):
        warnings.append("Actual sample gaps exceeded twice the requested interval.")
    if not samples:
        warnings.append("No successful samples; no memory result is available.")
    for phase in phases:
        if not phase["metric_samples"]:
            warnings.append(f"Phase {phase['id']} ({phase['name']}) has no usable memory samples.")
    if any(e["type"] == "diagnostic_start" for e in events):
        warnings.append("Diagnostic collection can perturb this capture; see diagnostic timestamps.")
    if manifest["scope"]["extra_roots"]:
        warnings.append("Explicit extra roots are counted in full, including any unrelated work.")
    if manifest["scope"].get("codex_bundle_prefix"):
        warnings.append("Verified Codex bundle processes are included even if detached; "
                        "bundle membership can also include shared or unrelated instances.")
    if ((manifest.get("workload") or {}).get("content") or {}).get("scenario") != "playback" and any(p["name"] == "work" and p["metrics_kib"]["codex_backend_rss_kib"] is not None
           and p["metrics_kib"]["codex_backend_rss_kib"]["sampled_max"] == 0 for p in phases):
        warnings.append("A work phase had no recognized Codex backend. Check process scope; "
                        "a phase marker alone does not prove Codex was exercised.")
    if manifest["workload_verification"] != "verified_automated":
        warnings.append("Workload equivalence and UI completion are unverified; no performance verdict.")
    else:
        warnings.append("Automated scenario completion verified; performance regressions are not assessed.")
    return {
        "schema_version": SCHEMA_VERSION,
        "label": manifest["label"],
        "target": manifest["target"],
        "context": manifest["context"],
        "outcome": manifest["outcome"],
        "reason": manifest.get("reason", ""),
        "performance_verdict": "not_assessed",
        "workload_verification": manifest["workload_verification"],
        "workload_sha256": (manifest.get("workload") or {}).get("sha256"),
        "sample_count": len(samples),
        "sample_errors": errors,
        "malformed_records": malformed,
        "sample_gaps_s": distribution(gaps),
        "collection_duration_s": distribution([s["collection_duration_s"] for s in samples]),
        "host_load_1m": distribution([s["host_load_1m"] for s in samples]),
        "root_absent_residual_rss_kib": distribution(
            [s["total_rss_kib"] for s in samples if not s["root_present"]]
        ),
        "phases": phases,
        "warnings": warnings,
        "limitations": manifest["limitations"],
    }


def mib(stats: dict | None, key: str = "median") -> str:
    return "unavailable" if stats is None else f"{stats[key] / 1024:.2f}"


def markdown(report: dict) -> str:
    lines = [
        f"# Memory capture: {report['label']}", "",
        f"Target: {report['target']}. Host context: {report['context']}. "
        f"Outcome: {report['outcome']}. Performance verdict: not assessed.", "",
        "All totals include the app, observed descendants, and explicit extra process trees. "
        "RSS is not deduplicated physical footprint. Maxima are sampled, not true peaks.", "",
        "Rows show median MiB, with sampled maximum total MiB. "
        "Root-absent and phase-boundary samples are counted but excluded from distributions.", "",
        "| Phase | Marker duration s | Metric/all samples | App | Descendants/extras | Codex backend subset | Total | Sampled max total |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for phase in report["phases"]:
        metrics = phase["metrics_kib"]
        name = f"{phase['id']}: {phase['name']}".replace("|", "\\|").replace("\n", " ")
        lines.append(
            f"| {name} | {phase['marker_duration_s']:.2f} | {phase['metric_samples']}/{phase['samples']} | "
            f"{mib(metrics['native_rss_kib'])} | {mib(metrics['descendants_rss_kib'])} | "
            f"{mib(metrics['codex_backend_rss_kib'])} | {mib(metrics['total_rss_kib'])} | "
            f"{mib(metrics['total_rss_kib'], 'sampled_max')} |"
        )
    if any(p["footprint_metrics_kib"]["total_footprint_kib"] for p in report["phases"]):
        lines.extend(["", "libproc footprint accounting (MiB). Totals sum per-process accounting; "
                      "they are not deduplicated group physical memory. Missing reads are not zero.", "",
                      "| Phase | App median | Total median | Sampled max total |",
                      "| --- | ---: | ---: | ---: |"])
        for phase in report["phases"]:
            metrics = phase["footprint_metrics_kib"]
            lines.append(f"| {phase['id']}: {phase['name']} | {mib(metrics['native_footprint_kib'])} | "
                         f"{mib(metrics['total_footprint_kib'])} | {mib(metrics['total_footprint_kib'], 'sampled_max')} |")
    lines.extend(["", "Samples with a recognized Codex child present:", "",
                  "Backend presence includes setup/shutdown and does not prove model generation. "
                  "This subset exposes execution costs when manual phase markers include an idle tail.", "",
                  "| Phase | Samples | App median MiB | Full total median MiB | Full total sampled max MiB |",
                  "| --- | ---: | ---: | ---: | ---: |"])
    for phase in report["phases"]:
        active = phase["codex_present"]
        if not active["samples"]:
            continue
        metrics = active["metrics_kib"]
        name = f"{phase['id']}: {phase['name']}".replace("|", "\\|").replace("\n", " ")
        lines.append(f"| {name} | {active['samples']} | {mib(metrics['native_rss_kib'])} | "
                     f"{mib(metrics['total_rss_kib'])} | {mib(metrics['total_rss_kib'], 'sampled_max')} |")
    lines.extend(["", "Approximate total CPU (ps moving estimate, not interval CPU time):", "",
                  "| Phase | Median % | Sampled max % |", "| --- | ---: | ---: |"])
    for phase in report["phases"]:
        cpu = phase["approx_cpu_pct"]
        name = f"{phase['id']}: {phase['name']}".replace("|", "\\|").replace("\n", " ")
        lines.append(f"| {name} | {cpu['median']:.2f} | {cpu['sampled_max']:.2f} |" if cpu
                     else f"| {name} | unavailable | unavailable |")
    lines.extend(["", f"Successful samples: {report['sample_count']}. "
                  f"Sample errors: {report['sample_errors']}.", "",
                  ("Phase windows follow app events; app-events.jsonl records submission-to-frame timing."
                   if report["workload_verification"] == "verified_automated" else
                   "Manual marker durations include operator timing; they do not verify turn latency."), "",
                  "Residual descendants/extras after root absence, sampled maximum MiB: "
                  f"{mib(report['root_absent_residual_rss_kib'], 'sampled_max')}.",
                  "", "Warnings:", ""])
    lines.extend(f"- {warning}" for warning in report["warnings"])
    lines.extend(["", "Measurement limits:", ""])
    lines.extend(f"- {limit}" for limit in report["limitations"])
    return "\n".join(lines) + "\n"


def save_report(directory: Path) -> dict:
    report = summarize(directory)
    write_json(directory / "report.json", report)
    (directory / "report.md").write_text(markdown(report))
    export_legacy(directory, report)
    return report


def export_legacy(directory: Path, report: dict) -> None:
    """Keep v3's familiar filenames/columns; raw events remain authoritative."""
    manifest = json.loads((directory / "manifest.json").read_text())
    total_fields = [
        "timestamp_utc", "elapsed_s", "phase", "process_count", "rss_total_mib",
        "rss_ui_mib", "rss_backend_mib", "rss_other_mib", "cpu_total_pct",
        "cpu_ui_pct", "cpu_backend_pct", "cpu_other_pct", "system_load_1m", "swap_used_mib",
        "phase_id", "root_present", "phase_boundary",
    ]
    process_fields = ["timestamp_utc", "elapsed_s", "phase", "pid", "ppid", "role", "name",
                      "rss_mib", "cpu_pct", "phase_id", "started"]
    phase_rows = defaultdict(list)
    with (directory / "totals.csv").open("w", newline="") as totals, \
            (directory / "processes.csv").open("w", newline="") as processes:
        tw = csv.DictWriter(totals, fieldnames=total_fields)
        pw = csv.DictWriter(processes, fieldnames=process_fields)
        tw.writeheader()
        pw.writeheader()
        with (directory / "events.jsonl").open() as events:
            for line in events:
                try:
                    sample = json.loads(line)
                except ValueError:
                    continue
                if not isinstance(sample, dict) or sample.get("type") != "sample":
                    continue
                members = sample["processes"]
                ui = [p for p in members if p["role"] in {"ui_main", "ui_helper", "app"}]
                backend = [p for p in members if p.get("backend")]
                rss_total = sample["total_rss_kib"] / 1024
                rss_ui = sum(p["rss_kib"] for p in ui) / 1024
                rss_backend = sum(p["rss_kib"] for p in backend) / 1024
                have_cpu = all("cpu_pct" in p for p in members)
                cpu_ui = sum(p["cpu_pct"] for p in ui) if have_cpu else None
                cpu_backend = sum(p["cpu_pct"] for p in backend) if have_cpu else None
                cpu_total = sample.get("total_cpu_pct")
                row = {
                    "timestamp_utc": sample["at"], "elapsed_s": sample["elapsed_s"],
                    "phase": sample["phase"], "phase_id": sample["phase_id"],
                    "process_count": len(members), "root_present": sample["root_present"],
                    "phase_boundary": sample["phase_boundary"],
                    "rss_total_mib": rss_total if sample["root_present"] else None,
                    "rss_ui_mib": rss_ui if sample["root_present"] else None,
                    "rss_backend_mib": rss_backend,
                    "rss_other_mib": rss_total - rss_ui - rss_backend,
                    "cpu_total_pct": cpu_total, "cpu_ui_pct": cpu_ui, "cpu_backend_pct": cpu_backend,
                    "cpu_other_pct": cpu_total - cpu_ui - cpu_backend if cpu_total is not None and have_cpu else None,
                    "system_load_1m": sample["host_load_1m"], "swap_used_mib": None,
                }
                tw.writerow(row)
                if sample["root_present"] and not sample["phase_boundary"]:
                    phase_rows[sample["phase"]].append(row)
                for p in members:
                    pw.writerow({
                        **{k: row[k] for k in ("timestamp_utc", "elapsed_s", "phase", "phase_id")},
                        **{k: p[k] for k in ("pid", "ppid", "role", "started")},
                        "name": Path(p["executable"]).name,
                        "rss_mib": p["rss_kib"] / 1024, "cpu_pct": p.get("cpu_pct"),
                    })
    statistics_by_name = {}
    for name, data in phase_rows.items():
        cpu = [r["cpu_total_pct"] for r in data if r["cpu_total_pct"] is not None]
        rss = [r["rss_total_mib"] for r in data]
        statistics_by_name[name] = {
            "sample_count": len(data), "first_elapsed_s": data[0]["elapsed_s"],
            "last_elapsed_s": data[-1]["elapsed_s"],
            "rss_total_mean_mib": statistics.mean(rss), "rss_total_median_mib": statistics.median(rss),
            "rss_total_peak_mib": max(rss), "cpu_mean_pct": statistics.mean(cpu) if cpu else None,
            "cpu_peak_pct": max(cpu) if cpu else None,
            "process_count_peak": max(r["process_count"] for r in data),
            "ui_rss_mean_mib": statistics.mean(r["rss_ui_mib"] for r in data),
            "backend_rss_mean_mib": statistics.mean(r["rss_backend_mib"] for r in data),
        }
    mode = {"smaller-talk": "smaller", "codex-app": "codex"}.get(manifest["target"], "custom")
    write_json(directory / "metadata.json", {
        **manifest, "export_format_version": 4,
        "mode": mode,
        "export_note": "Derived v3-style filenames/columns; swap is start/end context only. See manifest and raw events.",
    })
    write_json(directory / "summary.json", {
        "export_format_version": 4, "mode": mode,
        "valid_process_capture": any(p["metric_samples"] for p in report["phases"]),
        "total_samples": report["sample_count"], "phase_statistics": statistics_by_name,
        "outcome": report["outcome"], "performance_verdict": "not_assessed",
        "cautions": report["warnings"] + [
            "Legacy phase_statistics aggregates repeated names. Use report.json for separate phase sequences.",
            "rss_total_peak_mib is a sampled maximum; CPU is a ps moving estimate.",
            "Exports are regenerated after capture/report; they are not live CSV streams.",
        ],
    })


def compare(left: Path, right: Path) -> str:
    """Show complete process totals; never silently compare native-only numbers."""
    a, b = summarize(left), summarize(right)
    lines = [
        "Descriptive comparison only; workload equivalence is unverified.",
        f"A: {a['target']} / {a['label']} / {a['context']} / {a['outcome']}",
        f"B: {b['target']} / {b['label']} / {b['context']} / {b['outcome']}",
        "Total RSS includes execution backends on BOTH sides; shared pages can be double-counted.",
    ]
    if not a["workload_sha256"] or a["workload_sha256"] != b["workload_sha256"]:
        lines.append("WARNING: workload manifests are missing or different.")
    lines.extend(["", "| Phase | A total median MiB | B total median MiB | B minus A MiB |",
                  "| --- | ---: | ---: | ---: |"])
    phases_a = {(p["id"], p["name"]): p for p in a["phases"]}
    phases_b = {(p["id"], p["name"]): p for p in b["phases"]}
    for key in sorted(phases_a.keys() | phases_b.keys()):
        pa, pb = phases_a.get(key), phases_b.get(key)
        ma = pa["metrics_kib"]["total_rss_kib"] if pa else None
        mb = pb["metrics_kib"]["total_rss_kib"] if pb else None
        delta = f"{(mb['median'] - ma['median']) / 1024:.2f}" if ma and mb else "unavailable"
        name = f"{key[0]}: {key[1]}".replace("|", "\\|").replace("\n", " ")
        lines.append(f"| {name} | {mib(ma)} | {mib(mb)} | {delta} |")
    lines.extend(["", "Capture warnings (all attempts remain visible):"])
    lines.extend(f"- {side}: {warning}" for side, report in (("A", a), ("B", b))
                 for warning in report["warnings"])
    return "\n".join(lines) + "\n"

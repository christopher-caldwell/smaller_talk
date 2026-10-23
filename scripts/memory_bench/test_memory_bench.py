"""Accounting/integrity checks; no provider calls or desktop performance assertions."""

from __future__ import annotations

import json
import csv
import hashlib
import platform
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from contextlib import closing
from types import SimpleNamespace
from pathlib import Path
from unittest.mock import patch

from .cli import dispatch, legacy_main, main, resolve_root
from .core import Capture, Process, ProcessTree, parse_ps, read_processes
from .detection import codex_root_bundle, root_candidates
from .report import compare, save_report, summarize
from .launcher import configure_database, run as launch_benchmark
from .cli import parser

START = "Mon Oct 19 10:00:00 2026"
LATER = "Mon Oct 19 10:00:01 2026"


def process(pid=10, ppid=1, rss=100, executable="/Applications/Smaller Talk.app/Contents/MacOS/SmallerTalk", started=START):
    return Process(pid, ppid, started, rss, executable)


def rows(*processes):
    return {p.pid: p for p in processes}


class ProcessAccountingTests(unittest.TestCase):
    def test_ps_preserves_paths_with_spaces_and_rejects_bad_snapshots(self):
        parsed = parse_ps(
            " 10 1 Mon Oct 19 10:00:00 2026 1024 1.5 /Applications/Smaller Talk.app/Contents/MacOS/SmallerTalk\n"
            " 11 10 Mon Oct 19 10:00:01 2026 2048 2.5 /opt/bin/codex\n"
        )
        self.assertEqual(parsed[10].executable, "/Applications/Smaller Talk.app/Contents/MacOS/SmallerTalk")
        self.assertEqual(parsed[11].rss_kib, 2048)
        self.assertEqual(parsed[11].cpu_pct, 2.5)
        for bad in ("", "10 1 500 codex", "10 1 Mon Oct 19 10:00:00 2026 -1 0 codex"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_ps(bad)

    def test_counts_cli_launchers_tools_and_helpers_but_not_unrelated_codex(self):
        root = process()
        launcher = process(11, 10, 40, "/opt/bin/node")
        backend = process(12, 11, 200, "/opt/bin/codex")
        tool = process(13, 12, 30, "/usr/bin/git")
        renderer = process(14, 10, 80, "/Applications/Codex.app/Helper (Renderer)")
        unrelated = process(15, 1, 9000, "/opt/bin/codex")
        observed = ProcessTree(root, []).observe(rows(root, launcher, backend, tool, renderer, unrelated))
        self.assertEqual(observed["native_rss_kib"], 100)
        self.assertEqual(observed["descendants_rss_kib"], 350)
        self.assertEqual(observed["codex_backend_rss_kib"], 200)
        self.assertEqual(observed["total_rss_kib"], 450)
        self.assertNotIn(15, [p["pid"] for p in observed["processes"]])

    def test_observed_child_survives_reparenting_and_new_descendants_are_counted(self):
        root, child = process(), process(11, 10, 200, "/opt/bin/codex")
        tree = ProcessTree(root, [])
        tree.observe(rows(root, child))
        orphan = process(11, 1, 220, "/opt/bin/codex")
        tool = process(12, 11, 20, "/usr/bin/git")
        observed = tree.observe(rows(root, orphan, tool))
        self.assertEqual(observed["total_rss_kib"], 340)
        self.assertEqual(observed["disappeared"], [])

    def test_pid_reuse_does_not_transfer_ownership(self):
        root, child = process(), process(11, 10, 200, "/opt/bin/codex")
        tree = ProcessTree(root, [])
        tree.observe(rows(root, child))
        reused = process(11, 1, 9000, "/opt/bin/codex", LATER)
        observed = tree.observe(rows(root, reused))
        self.assertEqual(observed["total_rss_kib"], 100)
        self.assertEqual(observed["disappeared"], [[11, START]])
        reused_root = process(10, 1, 10000, "/Applications/Codex", LATER)
        observed = tree.observe(rows(reused_root, process(99, 10, 500)))
        self.assertFalse(observed["root_present"])
        self.assertIsNone(observed["native_rss_kib"])
        self.assertEqual(observed["processes"], [])

    def test_explicit_backend_tree_is_counted_once_even_when_also_descendant(self):
        root, backend = process(), process(11, 10, 200, "/opt/bin/codex")
        observed = ProcessTree(root, [backend]).observe(rows(root, backend))
        self.assertEqual(observed["total_rss_kib"], 300)
        self.assertEqual(len(observed["processes"]), 2)
        external = process(12, 1, 500, "/opt/bin/codex")
        observed = ProcessTree(root, [external]).observe(rows(root, external, process(13, 12, 50)))
        self.assertEqual(observed["total_rss_kib"], 650)

    def test_collector_running_under_codex_app_is_not_charged_as_app_work(self):
        root = process()
        backend = process(11, 10, 200, "/opt/bin/codex")
        observer = process(12, 11, 1000, "/opt/bin/python3")
        ps = process(13, 12, 50, "/bin/ps")
        tree = ProcessTree(root, [], observer)
        observed = tree.observe(rows(root, backend, observer, ps))
        self.assertEqual(observed["total_rss_kib"], 300)
        self.assertEqual({p["pid"] for p in observed["processes"]}, {10, 11})
        self.assertEqual({p[0] for p in observed["observer_excluded"]}, {12, 13})
        # Previously observed collector helpers remain excluded after reparenting.
        observed = tree.observe(rows(root, backend, observer, process(13, 10, 50, "/bin/ps")))
        self.assertEqual(observed["total_rss_kib"], 300)

    def test_observer_can_launch_a_target_without_excluding_the_target(self):
        observer = process(9, 1, 1000, "/opt/bin/python3")
        root = process(10, 9)
        backend = process(11, 10, 200, "/opt/bin/codex")
        observed = ProcessTree(root, [], observer).observe(rows(observer, root, backend))
        self.assertEqual(observed["total_rss_kib"], 300)

    def test_verified_bundle_detached_helpers_and_their_children_are_included(self):
        root = process(executable="/Applications/ChatGPT.app/Contents/MacOS/ChatGPT")
        detached = process(11, 1, 200, "/Applications/ChatGPT.app/Contents/Frameworks/Codex Helper")
        tool = process(12, 11, 20, "/usr/bin/git")
        unrelated = process(13, 1, 9000, "/Applications/Other.app/Contents/MacOS/codex")
        tree = ProcessTree(root, [], bundle_prefix="/Applications/ChatGPT.app/Contents/")
        observed = tree.observe(rows(root, detached, tool, unrelated))
        self.assertEqual(observed["total_rss_kib"], 320)
        self.assertEqual({p["pid"] for p in observed["processes"]}, {10, 11, 12})
        # A newly launched instance must not be adopted after the pinned root exits.
        later_helper = process(14, 1, 500, "/Applications/ChatGPT.app/Contents/Frameworks/Codex Helper")
        observed = tree.observe(rows(detached, tool, later_helper))
        self.assertEqual({p["pid"] for p in observed["processes"]}, {11, 12})


class OriginalWorkflowTests(unittest.TestCase):
    def test_codex_gui_uses_bundle_id_and_declared_main_not_app_display_name(self):
        info = {"CFBundleIdentifier": "com.openai.codex", "CFBundleExecutable": "ChatGPT"}
        with patch("memory_bench.detection.app_info", return_value=info):
            self.assertEqual(codex_root_bundle("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"), "/Applications/ChatGPT.app")
            self.assertIsNone(codex_root_bundle("/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex"))
            self.assertIsNone(codex_root_bundle("/Applications/ChatGPT.app/Contents/Frameworks/Codex Helper"))
        with patch("memory_bench.detection.app_info", return_value={"CFBundleIdentifier": "com.openai.chat", "CFBundleExecutable": "ChatGPT"}):
            self.assertIsNone(codex_root_bundle("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"))

    def test_multiple_gui_instances_require_explicit_selection(self):
        a = process(executable="/opt/one/smaller-talk-desktop")
        child = process(11, 10, executable="/opt/one/smaller-talk-desktop")
        b = process(12, 1, executable="/opt/two/smaller-talk-desktop")
        self.assertEqual(root_candidates("smaller-talk", rows(a, child, b)), [10, 12])
        with patch("memory_bench.cli.read_processes", return_value=rows(a, b)):
            with self.assertRaisesRegex(ValueError, "Multiple GUI roots"):
                resolve_root("smaller-talk", None, 1)

    def test_original_flags_route_to_new_collector_and_preserve_default_cadence(self):
        with patch("memory_bench.cli.main", return_value=0) as routed:
            self.assertEqual(legacy_main(["--mode", "smaller", "--out", "runs/one"]), 0)
            arguments = routed.call_args.args[0]
            self.assertIn("smaller-talk", arguments)
            self.assertEqual(arguments[arguments.index("--interval") + 1], "1.0")
            self.assertEqual(arguments[arguments.index("--phase") + 1], "launch")
            self.assertEqual(arguments[arguments.index("--output") + 1], "runs/one")
            legacy_main(["--mode", "codex", "--diagnose"])
            self.assertEqual(routed.call_args.args[0], ["discover", "--target", "codex-app"])
        with patch("memory_bench.cli.legacy_main", return_value=0) as compatibility:
            self.assertEqual(main(["--mode", "codex", "--diagnose"]), 0)
            compatibility.assert_called_once_with(["--mode", "codex", "--diagnose"])

    def test_imported_original_files_match_provenance_hashes(self):
        reference = Path(__file__).resolve().parents[2] / "benchmarks/reference/v3"
        provenance = json.loads((reference / "provenance.json").read_text())
        for name, expected in provenance["files_sha256"].items():
            self.assertEqual(hashlib.sha256((reference / name).read_bytes()).hexdigest(), expected)
        workload = json.loads((reference.parents[1] / "workloads/codex-manual-v1.json").read_text())
        self.assertEqual(workload["prompt"], (reference / "smaller_talk_workload_prompt.txt").read_text())


class CaptureIntegrityTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parents[2] / ".dev"
        parent.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="memory-bench-tests-", dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        for name in ("host_metadata", "host_memory", "command_metadata", "executable_metadata"):
            patcher = patch(f"memory_bench.core.{name}", return_value={})
            patcher.start()
            self.addCleanup(patcher.stop)

    def capture(self, name="run", reader=None, workload=None):
        capture = Capture(
            self.directory / name, 10, [], "smaller-talk", name, "busy", 0.05,
            workload=workload, reader=reader or (lambda: rows(process())),
        )
        self.addCleanup(capture.finish)
        return capture

    def test_append_only_events_and_existing_capture_cannot_be_overwritten(self):
        capture = self.capture()
        capture.mark("work", "Codex request sent manually")
        capture.sample()
        capture.finish("measurement_only", "done")
        with self.assertRaises(FileExistsError):
            self.capture()
        report = save_report(capture.output)
        self.assertEqual(report["sample_count"], 1)
        self.assertEqual(report["phases"][0]["name"], "work")
        self.assertEqual(report["performance_verdict"], "not_assessed")
        self.assertTrue(any("Busy" in warning for warning in report["warnings"]))
        self.assertTrue(any("no recognized Codex" in warning for warning in report["warnings"]))

    def test_collection_error_is_an_error_record_not_zero_memory(self):
        calls = iter([rows(process()), OSError("ps failed"), rows(process(rss=321))])

        def reader():
            value = next(calls)
            if isinstance(value, Exception):
                raise value
            return value

        capture = self.capture(reader=reader)
        capture.sample()
        capture.sample()
        capture.finish()
        report = summarize(capture.output)
        self.assertEqual(report["sample_errors"], 1)
        self.assertEqual(report["sample_count"], 1)
        self.assertEqual(report["phases"][0]["metrics_kib"]["total_rss_kib"]["min"], 321)

    def test_root_exit_remains_visible_without_turning_into_memory_savings(self):
        current = rows(process(), process(11, 10, 200, "/opt/bin/codex"))
        capture = self.capture(reader=lambda: current)
        capture.mark("work")
        capture.sample()
        current = rows(process(11, 1, 200, "/opt/bin/codex"))
        capture.sample()
        capture.finish("product_failure", "App crashed")
        report = summarize(capture.output)
        self.assertEqual(report["outcome"], "product_failure")
        self.assertEqual(report["phases"][0]["root_missing_samples"], 1)
        self.assertEqual(report["phases"][0]["metrics_kib"]["total_rss_kib"]["min"], 300)
        events = [json.loads(line) for line in (capture.output / "events.jsonl").read_text().splitlines()]
        self.assertFalse([e for e in events if e["type"] == "sample"][-1]["root_present"])

    def test_sample_straddling_phase_boundary_is_not_assigned_to_either_metric_window(self):
        capture = self.capture()

        def reader():
            capture.mark("work")
            return rows(process())

        capture.reader = reader
        capture.sample()
        capture.finish()
        report = summarize(capture.output)
        self.assertEqual(report["phases"][0]["boundary_samples"], 1)
        self.assertIsNone(report["phases"][0]["metrics_kib"]["total_rss_kib"])

    def test_repeated_phase_names_remain_separate(self):
        capture = self.capture()
        for phase in ("idle", "work", "idle"):
            capture.mark(phase)
            capture.sample()
        capture.finish()
        report = summarize(capture.output)
        self.assertEqual([p["name"] for p in report["phases"]], ["idle", "work", "idle"])
        self.assertEqual(len({p["id"] for p in report["phases"]}), 3)

    def test_phases_without_samples_are_reported_instead_of_disappearing(self):
        capture = self.capture()
        capture.mark("idle")
        capture.sample()
        capture.mark("work")
        capture.mark("recovery")
        capture.finish()
        report = summarize(capture.output)
        self.assertEqual([p["name"] for p in report["phases"]], ["idle", "work", "recovery"])
        self.assertEqual(report["phases"][1]["samples"], 0)
        self.assertTrue(any("work) has no usable" in warning for warning in report["warnings"]))

    def test_codex_present_subset_does_not_hide_execution_cost_in_a_long_manual_phase(self):
        current = rows(process(), process(11, 10, 900, "/opt/codex"))
        capture = self.capture(reader=lambda: current)
        capture.mark("work")
        capture.sample()
        current = rows(process())
        capture.sample()
        capture.sample()
        capture.finish()
        phase = summarize(capture.output)["phases"][0]
        self.assertEqual(phase["metrics_kib"]["total_rss_kib"]["median"], 100)
        self.assertEqual(phase["codex_present"]["samples"], 1)
        self.assertEqual(phase["codex_present"]["metrics_kib"]["total_rss_kib"]["median"], 1000)

    def test_malformed_tail_is_reported_and_original_trace_is_preserved(self):
        capture = self.capture()
        capture.sample()
        capture.finish()
        trace = capture.output / "events.jsonl"
        with trace.open("a") as stream:
            stream.write('{"type":')
        before = trace.read_bytes()
        report = save_report(capture.output)
        self.assertEqual(report["malformed_records"], 1)
        self.assertEqual(trace.read_bytes(), before)

    def test_workload_manifest_is_frozen_even_if_source_changes(self):
        workload = self.directory / "workload.json"
        workload.write_text('{"prompt":"fixed synthetic input"}')
        capture = self.capture(workload=workload)
        workload.write_text('{"prompt":"different input"}')
        capture.finish()
        manifest = json.loads((capture.output / "manifest.json").read_text())
        self.assertEqual(manifest["workload"]["content"]["prompt"], "fixed synthetic input")
        self.assertEqual(len(manifest["workload"]["sha256"]), 64)

    def test_thread_failure_cannot_be_reported_as_successful_measurement(self):
        capture = self.capture()
        original = capture.sample
        calls = 0

        def sample():
            nonlocal calls
            calls += 1
            if calls > 1:
                raise RuntimeError("collector bug")
            original()

        capture.sample = sample
        capture.start()
        self.assertTrue(capture.stop_event.wait(2))
        capture.finish("measurement_only")
        report = summarize(capture.output)
        self.assertEqual(report["outcome"], "harness_invalid")
        self.assertTrue(any("collector bug" in warning for warning in report["warnings"]))

    def test_commands_record_work_and_preserve_failure_classification(self):
        capture = self.capture()
        dispatch(capture, 'work "same model and effort"')
        self.assertEqual(capture.phase, "work")
        self.assertEqual(dispatch(capture, "fail response timed out"), ("product_failure", "response timed out"))
        self.assertEqual(dispatch(capture, "invalid automation missed button"), ("harness_invalid", "automation missed button"))
        self.assertEqual(dispatch(capture, "done"), ("measurement_only", ""))
        dispatch(capture, "post response finished")
        self.assertEqual(capture.phase, "post")
        self.assertEqual(dispatch(capture, "stop"), ("measurement_only", ""))
        with self.assertRaises(ValueError):
            dispatch(capture, "fail")

    def test_original_csv_filenames_include_cli_usage_cpu_and_no_command_arguments(self):
        root = Process(10, 1, START, 100, "/opt/smaller-talk-desktop", 2.5)
        backend = Process(11, 10, START, 900, "/opt/codex", 10.0)
        capture = self.capture(reader=lambda: rows(root, backend))
        capture.mark("work")
        capture.sample()
        capture.finish()
        save_report(capture.output)
        with (capture.output / "totals.csv").open() as source:
            total = next(csv.DictReader(source))
        self.assertEqual(float(total["rss_total_mib"]), 1000 / 1024)
        self.assertEqual(float(total["rss_backend_mib"]), 900 / 1024)
        self.assertEqual(float(total["cpu_total_pct"]), 12.5)
        with (capture.output / "processes.csv").open() as source:
            members = list(csv.DictReader(source))
        self.assertEqual({p["name"] for p in members}, {"smaller-talk-desktop", "codex"})
        self.assertNotIn("command", members[0])
        summary = json.loads((capture.output / "summary.json").read_text())
        self.assertTrue(summary["valid_process_capture"])
        self.assertEqual(summary["export_format_version"], 4)

    def test_footprint_requires_all_requested_pids_and_no_tool_errors(self):
        capture = self.capture()
        capture.sample()
        cases = [
            ({"processes": [{"pid": 10}], "errors": []}, True),
            ({"processes": [], "errors": []}, False),
            ({"processes": [{"pid": 10}], "errors": ["permission denied"]}, False),
            ({"processes": None, "errors": []}, False),
        ]
        for data, expected in cases:
            def run(command, **kwargs):
                Path(command[command.index("-j") + 1]).write_text(json.dumps(data))
                return SimpleNamespace(stdout="diagnostic", stderr="", returncode=0)

            with self.subTest(data=data), patch("memory_bench.core.subprocess.run", side_effect=run):
                self.assertEqual(capture.footprint()["usable"], expected)

    def test_root_identity_change_before_attachment_is_rejected(self):
        output = self.directory / "reused"
        with self.assertRaisesRegex(ValueError, "identity changed"):
            Capture(output, 10, [], "custom", "reused", "busy", 1,
                    reader=lambda: rows(process(started=LATER)), expected_identity=(10, START))
        self.assertFalse(output.exists())

    def test_comparison_uses_full_process_totals_for_both_apps_and_keeps_failures(self):
        a = self.capture("small", reader=lambda: rows(process(rss=100), process(11, 10, 900, "/opt/bin/codex")))
        b = self.capture("codex", reader=lambda: rows(process(rss=500), process(12, 10, 200, "/opt/bin/codex")))
        for capture in (a, b):
            capture.mark("work")
            capture.sample()
        a.finish()
        b.finish("product_failure", "incomplete answer")
        comparison = compare(a.output, b.output)
        self.assertIn("0.98 | 0.68 | -0.29", comparison)
        self.assertIn("product_failure", comparison)
        self.assertIn("workload manifests are missing or different", comparison)

    def test_invalid_duration_does_not_create_capture(self):
        output = self.directory / "invalid"
        for duration in ("nan", "-1", "inf"):
            with self.subTest(duration=duration):
                self.assertEqual(main(["record", "--target", "custom", "--pid", "10", "--label", "bad",
                                       "--duration", duration, "--output", str(output)]), 2)
                self.assertFalse(output.exists())


class ShortcutTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parents[2] / ".dev"
        parent.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="bench-shortcut-tests-", dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)

    @staticmethod
    def schema(path):
        with closing(sqlite3.connect(path)) as db, db:
            db.executescript("CREATE TABLE global_settings(id INTEGER PRIMARY KEY, payload TEXT); "
                             "CREATE TABLE app_state(id INTEGER PRIMARY KEY, payload TEXT);")

    def test_fresh_database_has_original_prompt_as_unsent_draft(self):
        database = self.base / "fresh.sqlite3"
        self.schema(database)
        prompt = "Explain this Rust function 🦀"
        configure_database(database, prompt, "chosen-model", "low")
        with closing(sqlite3.connect(database)) as db:
            session = json.loads(db.execute("SELECT payload FROM app_state").fetchone()[0])
        draft = session["new_chat"]
        self.assertEqual(draft["draft"]["text"], prompt)
        self.assertIsNone(draft["submitted"])
        self.assertEqual(draft["draft"]["selection_end"], len(prompt.encode()))
        self.assertEqual(session["active_tab"], draft["id"])

    def test_shortcut_builds_before_launch_and_pins_its_own_app_pid(self):
        output = self.base / "run"
        args = parser().parse_args(["bench", "--output", str(output), "--model", "chosen-model"])
        commands = []

        def execute(command, **kwargs):
            commands.append(command)
            if command[-1] == "db-migrate":
                self.schema(Path(kwargs["env"]["SMALLER_TALK_DATA_DIR"]) / "chats.sqlite3")
            return SimpleNamespace(returncode=0)

        app = SimpleNamespace(pid=424242, poll=lambda: None)
        with patch("memory_bench.launcher.platform.system", return_value="Darwin"), \
                patch("memory_bench.launcher.subprocess.run", side_effect=execute), \
                patch("memory_bench.launcher.subprocess.Popen", return_value=app) as opened, \
                patch("memory_bench.cli.main", return_value=0) as captured:
            self.assertEqual(launch_benchmark(args), 0)
        self.assertIn("--release", commands[0])
        self.assertEqual([cmd[-1] for cmd in commands[1:]], ["db-create", "db-migrate"])
        self.assertEqual(opened.call_args.kwargs["env"]["SMALLER_TALK_DATA_DIR"], str(output / "data"))
        arguments = captured.call_args.args[0]
        self.assertEqual(arguments[arguments.index("--target") + 1], "smaller-talk")
        self.assertEqual(arguments[arguments.index("--pid") + 1], "424242")
        self.assertFalse(json.loads((output / "launch.json").read_text())["prompt_submitted_automatically"])


@unittest.skipUnless(platform.system() == "Darwin", "Live collector requires macOS")
class MacOSCollectorProbe(unittest.TestCase):
    def test_real_child_allocation_release_and_exit_are_recorded(self):
        # 16 MiB briefly mapped in a disposable Python process, no UI/provider calls.
        worker = """
import json, mmap, subprocess, sys
child = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stdin.readline()'], stdin=subprocess.PIPE)
region = None
try:
    print(json.dumps({'child': child.pid}), flush=True)
    for command in sys.stdin:
        command = command.strip()
        if command == 'allocate':
            region = mmap.mmap(-1, 16 * 1024 * 1024)
            for _ in range(16): region.write(b'x' * (1024 * 1024))
        elif command == 'release':
            region.close()
            region = None
        elif command == 'child-exit':
            child.stdin.close()
            child.wait(timeout=5)
        elif command == 'quit':
            break
        print(json.dumps({'ack': command}), flush=True)
finally:
    if region is not None: region.close()
    if child.poll() is None:
        child.stdin.close()
        child.wait(timeout=5)
"""
        fixture = subprocess.Popen([sys.executable, "-u", "-c", worker], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, text=True)
        try:
            child_pid = json.loads(fixture.stdout.readline())["child"]
            initial = read_processes()
            tree = ProcessTree(initial[fixture.pid], [])
            idle = tree.observe(initial)
            self.assertEqual({p["pid"] for p in idle["processes"]}, {fixture.pid, child_pid})
            for command in ("allocate", "release", "child-exit"):
                fixture.stdin.write(command + "\n")
                fixture.stdin.flush()
                self.assertEqual(json.loads(fixture.stdout.readline())["ack"], command)
                observed = tree.observe(read_processes())
                self.assertTrue(observed["root_present"])
                self.assertGreater(observed["native_rss_kib"], 0)
                if command == "child-exit":
                    self.assertEqual([p["pid"] for p in observed["processes"]], [fixture.pid])
                    self.assertEqual(observed["disappeared"][0][0], child_pid)
                else:
                    self.assertEqual(len(observed["processes"]), 2)
            # Residency changes under compression are not treated as a performance assertion.
        finally:
            fixture.stdin.close()
            try:
                fixture.wait(timeout=5)
            except subprocess.TimeoutExpired:
                fixture.kill()
                fixture.wait(timeout=5)
            fixture.stdout.close()
        self.assertEqual(fixture.returncode, 0)


if __name__ == "__main__":
    unittest.main()

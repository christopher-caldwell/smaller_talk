"""Scenario integrity checks: corruption, incomplete attempts, transport, and real OS accounting."""
from __future__ import annotations

import ctypes
import io
import json
import os
import platform
import sqlite3
import subprocess
import tempfile
import unittest
from contextlib import closing
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from .cli import main, parser
from .core import Capture
from .macos import FootprintReader, RUsageV0
from .report import summarize
from .scenarios import AppEvents, HarnessFailure, ScenarioFailure, capture_issues, paired_comparison, seed_history, seed_workspace, suite_report, verify_database
from .test_memory_bench import process, rows


class ScenarioTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parents[2] / ".dev"
        parent.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(prefix="automated-bench-test-", dir=parent)
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        history = patch("memory_bench.scenarios.save_results", return_value=self.base / "results.json")
        history.start()
        self.addCleanup(history.stop)

    def database(self, name="database.sqlite3"):
        path = self.base / name
        migrations = Path(__file__).resolve().parents[2] / "crates/database/migrations"
        with closing(sqlite3.connect(path)) as database, database:
            for migration in sorted(migrations.glob("*.sql")):
                database.executescript(migration.read_text())
        return path

    def complete_turn(self):
        path = self.database()
        with closing(sqlite3.connect(path)) as database, database:
            database.execute("INSERT INTO chats(id,title,created_at) VALUES('chat','Test',1)")
            database.execute("INSERT INTO messages VALUES('user','chat','user','prompt',1)")
            database.execute("INSERT INTO messages VALUES('assistant','chat','assistant','answer',2)")
            database.execute("INSERT INTO final_answers VALUES('assistant','answer')")
            database.execute("INSERT INTO turns(id,chat_id,user_message_id,settings,workspace,status) VALUES('turn','chat','user','{}','workspace','completed')")
        return path

    def test_verification_requires_exact_fixture_bytes_and_completed_turn(self):
        path = self.complete_turn()
        self.assertTrue(verify_database(path, "chat", "prompt", "answer")["fixture_exact_match"])
        for prompt, answer in (("wrong", "answer"), ("prompt", "wrong")):
            with self.assertRaises(ScenarioFailure):
                verify_database(path, "chat", prompt, answer)
        with closing(sqlite3.connect(path)) as database, database:
            database.execute("UPDATE turns SET status='failed',error='test failure'")
        with self.assertRaises(ScenarioFailure):
            verify_database(path, "chat", "prompt", "answer")

    def test_final_answer_corruption_is_detected_even_with_correct_visible_message(self):
        path = self.complete_turn()
        with closing(sqlite3.connect(path)) as database, database:
            database.execute("UPDATE final_answers SET text='corrupted'")
        with self.assertRaises(ScenarioFailure):
            verify_database(path, "chat", "prompt", "answer")

    def test_reset_history_is_identical_and_does_not_grow_across_runs(self):
        first = seed_history(self.database("first.sqlite3"), "# Fixed answer\n\n```rust\nfn x() {}\n```\n")
        second = seed_history(self.database("second.sqlite3"), "# Fixed answer\n\n```rust\nfn x() {}\n```\n")
        self.assertEqual(first, second)
        self.assertEqual(first["messages"], 200)

    def test_workspace_stops_parent_git_discovery_and_has_identical_revision(self):
        revisions = []
        for name in ("first", "second"):
            base = self.base / name
            workspace = base / "workspace"
            workspace.mkdir(parents=True)
            revisions.append(seed_workspace(workspace, base))
            root = subprocess.run(["git", "-C", str(workspace), "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
            self.assertEqual(Path(root), workspace)
        self.assertEqual(revisions[0], revisions[1])

    def test_missing_phase_or_incomplete_footprint_cannot_pass_capture_quality(self):
        phases = [{"name": name, "metric_samples": 3, "footprint_samples": {"total_footprint_kib": 3}}
                  for name in ("idle", "work", "post")]
        report = {"phases": phases, "sample_errors": 0, "malformed_records": 0}
        self.assertEqual(capture_issues(report, "live", 0), [])
        self.assertIn("missing phase closed-baseline", capture_issues(report, "playback", 1))
        phases[1]["footprint_samples"]["total_footprint_kib"] = 0
        self.assertIn("insufficient samples in work", capture_issues(report, "live", 0))

    def test_transport_preserves_partial_records_and_matches_action_ids(self):
        path = self.base / "events.jsonl"
        app = SimpleNamespace(poll=lambda: None, stdin=io.StringIO())
        events = AppEvents(path, app)
        value = json.dumps({"event": "view_rendered", "data": {"request": 7}})
        path.write_text(value[:10])
        self.assertEqual(events.read(), [])
        with path.open("a") as stream:
            stream.write(value[10:] + "\n")
        self.assertEqual(events.wait("view_rendered", 0.1, 7)["data"]["request"], 7)
        events.send("scroll", end=True)
        self.assertEqual(json.loads(app.stdin.getvalue())["command"], "scroll")
        self.assertEqual(len(events.events), 1)

    def test_transport_reports_app_exit_error_and_malformed_evidence(self):
        path = self.base / "events.jsonl"
        events = AppEvents(path, SimpleNamespace(poll=lambda: 4, returncode=4))
        with self.assertRaisesRegex(ScenarioFailure, "exited"):
            events.wait("ui_ready", 0.1)
        with self.assertRaisesRegex(HarnessFailure, "benchmark protocol"):
            events.wait("boot_waiting", 0.1)
        path.write_text("not-json\n")
        with self.assertRaises(HarnessFailure):
            events.read()
        path.write_text(json.dumps({"event": "error", "data": {"message": "failed"}}) + "\n")
        events.offset = 0
        with self.assertRaisesRegex(ScenarioFailure, "failed"):
            events.read()

    def test_comparison_uses_independent_pairs_and_keeps_failure_denominator(self):
        attempts = []
        for pair in range(1, 7):
            for build, value in (("baseline", 100), ("candidate", 105)):
                attempts.append({"kind": "playback", "pair": pair, "build": build,
                                 "outcome": "completed" if pair < 6 else "product_failure",
                                 "metrics": {"work.total_footprint_sampled_max_mib": value}})
        result = paired_comparison(attempts, 1)
        self.assertEqual(result["attempted_pairs"], 6)
        self.assertEqual(result["complete_pairs"], 5)
        metric = result["metrics"]["work.total_footprint_sampled_max_mib"]
        self.assertEqual(metric["paired_mean_delta_mib"], 5)
        self.assertEqual(metric["exploratory_bootstrap_95_percent_interval_mib"], [5, 5])
        self.assertEqual(result["verdict"], "not_assessed")
        self.assertIsNone(paired_comparison(attempts[:2], 1)["metrics"]["work.total_footprint_sampled_max_mib"]["exploratory_bootstrap_95_percent_interval_mib"])

    def test_empty_and_interrupted_suites_cannot_report_completion(self):
        args = parser().parse_args(["bench", "--offline"])
        result = suite_report(self.base, [], args, {"expected_attempts": 3})
        self.assertEqual(result["outcome"], "incomplete")
        self.assertEqual(result["groups"], {})

    def test_default_is_automated_and_includes_live_codex_but_manual_is_preserved(self):
        with patch("memory_bench.scenarios.run", return_value=0) as automated:
            self.assertEqual(main(["bench"]), 0)
            self.assertFalse(automated.call_args.args[0].offline)
        with patch("memory_bench.launcher.run", return_value=0) as manual:
            self.assertEqual(main(["bench", "--manual"]), 0)
            self.assertEqual(main(["bench", "--duration", "2"]), 0)
            self.assertEqual(manual.call_count, 2)

    def test_invalid_scenario_options_do_not_build_or_create_output(self):
        for option in (["--repeats", "0"], ["--cycles", "-1"], ["--settle-seconds", "0.01"]):
            destination = self.base / "absent"
            with patch("memory_bench.scenarios.subprocess.run") as execute:
                self.assertEqual(main(["bench", "--output", str(destination), *option]), 2)
                execute.assert_not_called()
                self.assertFalse(destination.exists())

    def test_failed_build_writes_failure_evidence_instead_of_empty_success(self):
        destination = self.base / "failed-build"
        with patch("memory_bench.scenarios.prepare_binaries", side_effect=subprocess.CalledProcessError(7, ["cargo", "build"])):
            self.assertEqual(main(["bench", "--output", str(destination)]), 2)
        result = json.loads((destination / "suite.json").read_text())
        self.assertEqual(result["outcome"], "harness_invalid")
        self.assertEqual(result["trials"], [])
        self.assertIn("Preparation failed", (destination / "report.md").read_text())

    def test_partial_footprint_reads_are_unavailable_not_smaller_totals(self):
        def footprint(pid):
            if pid == 11:
                raise OSError("permission denied")
            return {"footprint_kib": 50, "rusage_start_abstime": 1}
        with patch("memory_bench.core.host_metadata", return_value={}), \
                patch("memory_bench.core.host_memory", return_value={}):
            capture = Capture(self.base / "capture", 10, [], "custom", "probe", "busy", 1,
                              reader=lambda: rows(process(), process(11, 10, 200)), footprint_reader=footprint)
            self.addCleanup(capture.finish)
            capture.mark("work")
            capture.sample()
            capture.finish()
        report = summarize(capture.output)
        phase = report["phases"][0]
        self.assertEqual(phase["metrics_kib"]["total_rss_kib"]["median"], 300)
        self.assertIsNone(phase["footprint_metrics_kib"]["total_footprint_kib"])
        self.assertEqual(phase["footprint_metrics_kib"]["native_footprint_kib"]["median"], 50)
        self.assertTrue(any("incomplete footprint" in warning for warning in report["warnings"]))

    def test_final_failure_assessment_is_append_only_and_does_not_extend_measurement(self):
        with patch("memory_bench.core.host_metadata", return_value={}), \
                patch("memory_bench.core.host_memory", return_value={}):
            capture = Capture(self.base / "capture", 10, [], "custom", "probe", "busy", 1,
                              reader=lambda: rows(process()))
            self.addCleanup(capture.finish)
            capture.mark("post")
            capture.sample()
            capture.finish("completed", "Work completed")
        previous = (capture.output / "events.jsonl").read_text()
        with patch("memory_bench.core.time.monotonic", return_value=capture.origin + 100):
            capture.assess("product_failure", "App shutdown failed")
        self.assertTrue((capture.output / "events.jsonl").read_text().startswith(previous))
        final = json.loads((capture.output / "events.jsonl").read_text().splitlines()[-1])
        self.assertEqual(final["type"], "assessment")
        self.assertEqual(final["outcome"], "product_failure")
        report = summarize(capture.output)
        self.assertEqual(report["outcome"], "product_failure")
        self.assertLess(report["phases"][0]["marker_duration_s"], 1)
        with self.assertRaises(RuntimeError):
            capture.assess("completed", "Cannot overwrite the assessment")

    def test_footprint_identity_change_cannot_be_attributed_to_old_process(self):
        starts = iter([1, 2])
        with patch("memory_bench.core.host_metadata", return_value={}), \
                patch("memory_bench.core.host_memory", return_value={}):
            capture = Capture(self.base / "capture", 10, [], "custom", "probe", "busy", 1,
                              reader=lambda: rows(process()), footprint_reader=lambda _: {"footprint_kib": 50, "rusage_start_abstime": next(starts)})
            self.addCleanup(capture.finish)
            capture.sample()
            capture.sample()
            capture.finish()
        samples = [json.loads(line) for line in (capture.output / "events.jsonl").read_text().splitlines() if json.loads(line)["type"] == "sample"]
        self.assertEqual(samples[0]["total_footprint_kib"], 50)
        self.assertIsNone(samples[1]["total_footprint_kib"])
        self.assertIn("identity changed", samples[1]["processes"][0]["footprint_error"])

    @unittest.skipUnless(platform.system() == "Darwin", "Darwin libproc")
    def test_real_libproc_layout_and_identity_are_stable(self):
        self.assertEqual(ctypes.sizeof(RUsageV0), 96)
        reader = FootprintReader()
        first, second = reader(os.getpid()), reader(os.getpid())
        self.assertGreater(first["footprint_kib"], 0)
        self.assertEqual(first["rusage_start_abstime"], second["rusage_start_abstime"])
        with self.assertRaises(OSError):
            reader(2_147_483_647)

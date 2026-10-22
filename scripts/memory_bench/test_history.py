"""Historical results preserve attribution, failure counts, and immutable evidence."""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from .cli import main
from .core import Capture, write_json
from .history import compact_results, save_results
from .test_memory_bench import process, rows


class HistoryTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parents[2] / ".dev"
        parent.mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="benchmark-history-test-", dir=parent)
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.suite = self.base / "suite"
        self.suite.mkdir()
        self.results = self.base / "git-results"

    def make_suite(self, failed_second=False, footprint=True):
        attempts = []
        for index, multiple in enumerate((1, 2)):
            trial = self.suite / f"trial-{index}"
            trial.mkdir()
            workload = trial / "workload.json"
            write_json(workload, {"scenario":"playback", "model":"fixture-model", "prompt":"PRIVATE_RESPONSE_SENTINEL",
                                  "provider_binary":"/private/sentinel/codex", "prompt_sha256":"prompt-hash"})
            current = rows(process(rss=100 * multiple * 1024), process(11,10,900 * multiple * 1024,"/private/sentinel/codex"))
            def usage(pid, scale=multiple):
                return {"footprint_kib": (50 if pid == 10 else 500) * scale * 1024, "rusage_start_abstime":pid}
            with patch("memory_bench.core.host_metadata", return_value={"machine":"arm64","cpu_count":8,"hostname":"PRIVATE_HOST"}), \
                    patch("memory_bench.core.host_memory", return_value={}):
                capture = Capture(trial / "capture", 10, [], "smaller-talk", "test", "busy", 1,
                                  workload, reader=lambda:current, footprint_reader=usage if footprint else None)
                capture.mark("work")
                capture.sample()
                current = rows(process(rss=500 * multiple * 1024))
                capture.mark("retained-1")
                capture.sample()
                outcome = "product_failure" if index == 1 and failed_second else "completed"
                capture.manifest["workload_verification"] = "verified_automated"
                capture.finish(outcome)
            attempts.append({"kind":"playback","build":"candidate","pair":index + 1,"directory":str(trial),
                             "outcome":outcome,"binary_sha256":"binary-hash"})
        write_json(self.suite / "suite.json", {"run_id":"fixed-test-run","started_at":"2026-10-22T12:00:00+00:00",
                   "source_revision":{"value":"revision"},"source_branch":{"value":"feature"},
                   "source_status":{"value":"dirty"},"context":"busy","outcome":"failed" if failed_second else "completed",
                   "expected_attempts":2,"trials":attempts,"interval_s":1,"label":"test"})

    def test_app_and_backend_growth_stay_distinct_without_leaking_raw_content(self):
        self.make_suite()
        result = compact_results(self.suite)
        group = result["results"][0]
        work, retained = group["phases"]
        self.assertEqual(work["app_rss_median_mib"], 150)
        self.assertEqual(work["group_rss_peak_mib"], 1500)
        self.assertEqual(work["codex_samples"], 2)
        self.assertEqual(work["non_app_process_samples"], 2)
        self.assertEqual(retained["app_rss_median_mib"], 750)
        self.assertEqual(retained["codex_samples"], 0)
        self.assertEqual(retained["non_app_process_samples"], 0)
        self.assertEqual(group["attempts"], {"completed":2})
        self.assertEqual(result["source"], {"revision":"revision","branch":"feature","dirty":True})
        encoded = json.dumps(result)
        for private in ("PRIVATE_RESPONSE_SENTINEL", "PRIVATE_HOST", "/private/sentinel", str(self.base)):
            self.assertNotIn(private, encoded)

    def test_failed_attempts_are_counted_and_do_not_enter_successful_aggregates(self):
        self.make_suite(failed_second=True)
        group = compact_results(self.suite)["results"][0]
        self.assertEqual(group["attempts"], {"completed":1,"product_failure":1})
        self.assertEqual(group["phases"][0]["app_rss_median_mib"], 100)
        self.assertEqual(group["phases"][0]["runs"], 1)

    def test_missing_footprint_remains_null(self):
        self.make_suite(footprint=False)
        phase = compact_results(self.suite)["results"][0]["phases"][0]
        self.assertIsNone(phase["app_footprint_median_mib"])
        self.assertGreater(phase["app_rss_median_mib"], 0)

    def test_saved_records_are_idempotent_and_never_overwritten(self):
        self.make_suite()
        path = save_results(self.suite, self.results)
        before = path.read_bytes()
        self.assertEqual(save_results(self.suite, self.results), path)
        metadata = json.loads((self.suite / "suite.json").read_text())
        metadata["context"] = "controlled"
        write_json(self.suite / "suite.json", metadata)
        with self.assertRaises(FileExistsError):
            save_results(self.suite, self.results)
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(list(self.results.glob(".result-*")), [])

    def test_cli_exports_results_without_rerunning_the_benchmark(self):
        self.make_suite()
        self.assertEqual(main(["save-results",str(self.suite),"--results-dir",str(self.results)]), 0)
        self.assertEqual(len(list(self.results.glob("*.json"))), 1)

    def test_preparation_failure_with_no_samples_is_preserved(self):
        write_json(self.suite / "suite.json", {"run_id":"failed","started_at":"2026-10-22T12:00:00Z",
                   "outcome":"harness_invalid","context":"busy","trials":[],"expected_attempts":4})
        record = compact_results(self.suite)
        self.assertEqual(record["outcome"], "harness_invalid")
        self.assertEqual(record["expected_attempts"], 4)
        self.assertEqual(record["results"], [])

    def test_legacy_capture_does_not_claim_collector_revision_as_app_source(self):
        self.make_suite()
        record = compact_results(self.suite / "trial-0/capture")
        self.assertEqual(record["results"][0]["scenario"], "manual")
        self.assertIsNone(record["source"]["revision"])
        self.assertEqual(record["results"][0]["binary_sha256"], None)
        self.assertEqual(save_results(self.suite / "trial-0", self.results),
                         save_results(self.suite / "trial-0/capture", self.results))

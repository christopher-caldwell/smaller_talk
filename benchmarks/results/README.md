# Memory results history

Each JSON file is one immutable run summary. `just bench` writes a record here
automatically when a suite finishes, fails, or is interrupted. `just bench
--manual` exports its capture after sampling stops. New records are ordinary Git
changes to include in a commit; the benchmark does not create commits itself.

Full reports, samples, databases, responses, and copied binaries stay local under
`.dev/memory-bench/`. The earlier narrative reports in `docs/benchmarks/` are also
ignored. This directory contains results and comparison metadata only.

`comparisons/` contains selected measurements and links to the source result
records. The [history driver control](comparisons/2026-10-22-history-driver-control.json)
compares ordinary UI interaction with the native driver using the same binary
and history fixture. Its new trials use fixed 30-second observation windows;
the record includes the final 10-second medians and a longer ordinary-app idle
check. These are exploratory observations from a busy host.

The [context-retention investigation](comparisons/2026-10-22-history-context-retention.json)
records the Taffy context cleanup backport, before/after drop tests, paired
measurements, separate heap diagnostics, ordinary UI validation, and a real
Codex job. It distinguishes the final fix from the earlier virtualization
experiment, which was not retained.

The [two-drawable follow-up](comparisons/2026-10-22-two-metal-drawables.json)
records the macOS renderer's surface-memory reduction, three paired history
runs, a normal-use control with longer settling, native resize/copy checks, and
a real Codex job. It also preserves the inconclusive small-chat collection
experiment that was reverted, and records the remaining transient history peak.

The history includes the existing manual run and automated validation attempts,
including incomplete/failed attempts. Re-export an existing suite or capture with:

```sh
just memory-bench save-results .dev/memory-bench/RUN
```

Exports are idempotent. Different contents cannot replace a saved run. Filenames
and timestamps use UTC. Missing historical metadata and unavailable measurements
remain `null`; older runs are not assigned the current branch or commit.

Each result separates:

- `app_*`: Smaller Talk's own memory.
- `group_*`: Smaller Talk plus its observed execution processes and helpers.
- `codex_samples`: usable samples containing a recognized Codex backend.
- `non_app_process_samples` and `other_process_names`: other observed processes,
  including Git helpers or the local playback fixture.

For repeated trials, memory values are medians of individual run summaries;
peaks are medians of the sampled maxima, not sums of separate process maxima.
Sample counts sum across completed trials. Failed attempts remain in `attempts`
but do not enter successful memory aggregates. Footprint totals sum per-process
accounting and are not deduplicated group physical memory. Polling can miss brief
processes; zero observed backend samples does not prove no backend ever existed.

Compare matching workload/fixture hashes, workspace, timing, provider settings,
host conditions, and measurement methods. Busy-system observations are not
regression verdicts. The manual run's broad work marker includes an idle tail;
its phase medians are not equivalent to automated execution windows.

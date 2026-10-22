# Memory benchmark framework

The framework benchmarks **Smaller Talk and its execution processes**, including
its own Codex CLI and tools. It does not launch or benchmark the Codex desktop app.
The old multi-app collector interface remains available for compatibility.

The automated runner uses repeatable native-app scenarios, semantic boundaries,
fresh databases, pinned release binaries, full process accounting, and persisted
output checks. It keeps deterministic playback separate from a real Codex job.
Reports are descriptive; busy-system runs do not establish performance regressions.

## Run the automated suite

Python 3.10+ on macOS is sufficient; there are no package dependencies.

For the usual Smaller Talk benchmark, run:

```sh
just bench
```

No typing or clicking is required. The command:

1. Builds once before measurement and copies the release executable into the run
   directory so later builds cannot replace the measured artifact.
2. Runs three independent playback launches. Each uses the real Codex adapter
   with a local protocol fixture, the original prompt, and the same recorded
   4,419-byte answer in 56 chunks on a fixed schedule. This exercises the normal
   UI send handler, queue, persistence, streaming, Markdown, and GPUI rendering.
3. In each playback launch, opens a fixed 200-message history, renders both ends,
   and closes it three times. The database stays bounded. Closed-tab baseline and
   retained phases make first-use and repeated-use retention visible.
4. Runs one real Codex job with the original prompt, including the app's CLI and
   all observed owned helpers. This submits a real model request using your
   installed, signed-in CLI. Live results remain separately labeled.
5. Verifies persisted completion, playback response/final-answer hashes, the
   completed view frame, execution process cleanup, and clean app shutdown.
   Only windows/processes created by this runner are closed.

Results are written under `.dev/memory-bench/<timestamp>-suite/`. Open `report.md`
for the suite summary or `suite.json` for individual attempts and metrics. Every
trial retains its database, binary hash, `app-events.jsonl`, `completion.json`,
response, process capture, logs, and cleanup evidence. Failed attempts remain in
the report and the command exits nonzero. Ctrl+C preserves an incomplete attempt.

Compact historical results are written automatically to
[`benchmarks/results/`](../benchmarks/results/README.md), including failed and
interrupted attempts. These records contain measurements, app/backend sample
counts, workload and binary hashes, source revision/dirty state, and host context.
App-only retention is separate from the expected full-process spike during Codex
execution. New records are ordinary Git changes; the runner never commits other
work. Full reports, raw samples, responses, and binaries remain local and ignored.
Export an older suite or capture without running it again:

```sh
just memory-bench save-results .dev/memory-bench/RUN
```

The runner creates fresh app data and an identical empty Git repository per trial;
the fixed commit stops Git discovery from inheriting the source checkout's status.
Existing app data
is untouched. Live Codex inherits host provider/MCP configuration and ancestor
instructions. Its actual execution settings, CLI version, executable hash, output
size, and observed CLI processes are recorded. A fixed prompt does not make live
model output deterministic.

Useful options:

```sh
just bench --model gpt-6-luna --effort low
just bench --offline
just bench --offline --repeats 1 --cycles 1
just bench --context controlled --repeats 10
just bench --binary /absolute/path/to/benchmark-capable-release
just bench --baseline-binary /absolute/path/to/previous/benchmark-capable-release
just bench --help
```

`--offline` deliberately omits real CLI costs. `--repeats` controls independent
playback launches; live remains one descriptive job per build. With a baseline
binary, each playback pair contains both builds in a recorded randomized order,
each with fresh data. Both binaries must implement benchmark protocol 1; old
releases without the driver are reported as failed attempts. Binaries are copied
before capture. The comparison records paired deltas and exploratory bootstrap
intervals with at least five complete pairs. It never declares a release gate,
and lists incomplete/failed pairs explicitly. Identical binary hashes are useful
for testing the comparison machinery, not demonstrating an optimization.

The one-second cadence and three-second fixed observation windows are defaults,
not statistical guarantees. `--interval` and `--settle-seconds` can be increased;
settling must cover at least three sample intervals. A warmup waits for initial
provider-discovery processes to exit before the idle observation window. Each
scored phase requires at least two RSS and complete footprint samples. Maxima are
sampled maxima. The runner finishes its builds before capture and does not launch
a profiler during a trial. Other host work remains outside its control.

The explicit `benchmark <run-directory>` desktop command is a local stdin/event
file transport used by this runner. Ordinary launches do not start it. A launch
gate lets the collector attach before database/provider/UI initialization. This
still excludes executable loading and does not flush OS caches. App events mark
initialization, UI ready, submission, execution start, first content, execution
finish, and a GPUI post-frame callback. That callback proves the view frame ran;
it is not an assertion about every displayed pixel or UI accessibility.

Automated runs read `ri_phys_footprint` with `libproc` without suspending the app.
Reports keep per-process footprint and RSS separate. Footprint totals sum process
accounting; they are **not deduplicated physical memory for the group**. Failed
reads leave a total unavailable, never zero. Reads are sequential, and polling
can miss short-lived children. Intrusive `footprint` snapshots and Instruments
remain separate diagnostics. The regular suite requires no Xcode test target or
Instruments installation.

## Original interactive workflow

```sh
just bench --manual
```

This builds a fresh release app with the original prompt prefilled but unsent.
In the benchmark terminal:

1. Confirm sign-in and model in the app. Type `idle`, then wait about 30 seconds.
2. Type `work`, then press Send in Smaller Talk.
3. When the response finishes, type `post`, wait about 30 seconds, then type `stop`.

Results are saved automatically under `.dev/memory-bench/<timestamp>-smaller-talk/capture/`.
The app remains open for inspection; close that benchmark window when finished.
The launcher uses Codex, disables Claude and automatic shortening in its new database,
and inherits the host's provider configuration. Existing app data is untouched.
The run is marked busy by default. `just bench --duration 2` retains the original
timed, unsent startup check. Manual startup can begin before collector attachment;
it does not use the automated launch gate. The low-level `record` collector only
observes processes; it never submits prompts or closes apps.

The original command line and phase commands are supported:

```sh
python3 scripts/smaller_talk_benchmark.py --mode smaller \
  --out .dev/memory-bench/smaller-01 --context busy \
  --workload benchmarks/workloads/codex-manual-v1.json
```

Use `--mode codex` for the other app. `just memory-bench --mode smaller --out ...`
also accepts these flags. Without `--pid`, start the monitor before launching the
app in another Terminal: it waits up to 60 seconds for one verified GUI root and
refuses to guess between multiple roots. Override with `--wait-timeout`. No
zero-memory result is created while waiting. Sampling begins after root detection
and metadata collection; this does not capture every early startup allocation.
`--duration` measures time after attachment, separately from the root wait.

The original diagnose command is supported too:

```sh
python3 scripts/smaller_talk_benchmark.py --mode codex --diagnose
```

The newer explicit capture interface is:

```sh
just memory-bench discover
```

Choose the main app PID from the executable paths, then attach explicitly:

```sh
just memory-bench record --target smaller-talk --pid 12345 \
  --label candidate-codex-work --context busy \
  --workload benchmarks/workloads/codex-manual-v1.json
```

For the comparison app, use its main PID:

```sh
just memory-bench record --target codex-app --pid 23456 \
  --label codex-app-work --context busy \
  --workload benchmarks/workloads/codex-manual-v1.json
```

GUI roots are verified: Codex requires `CFBundleIdentifier=com.openai.codex` and
the bundle's declared main executable, including installations named ChatGPT.app.
`discover` separately lists Codex executables to help inspect backend placement. Do not
select a CLI PID as the main app PID. Some installations host Codex inside
`ChatGPT.app`: measuring that main process includes its other chats, services,
and tasks. The collector cannot apportion shared process memory to one task.

Every observed descendant is counted, regardless of its executable name. Do not
globally sum processes named `codex`: other terminals and tasks may use independent
instances. As in v3, detached executables inside the verified Codex bundle, plus
their descendants, are also counted. They are marked by bundle-path association;
that can include shared or unrelated instances using the same bundle. Enrollment
of new bundle-associated processes stops when the pinned GUI root exits.
If another required daemon is outside these groups, include it explicitly:

```sh
just memory-bench record --target smaller-talk --pid 12345 \
  --include-pid 34567 --label candidate-with-external-backend
```

This includes that extra root's entire observed tree, once per process identity.
Shared backends are charged in full and flagged in the report. Document their
other work; including a shared daemon does not establish task-specific ownership.
An existing child that detached before capture began needs explicit inclusion.

The collector and its observed helper processes are excluded, including when it
is launched from inside the measured Codex app. Their pinned identities are
recorded. Parent shells and shared tool hosts remain in scope if they belong to
the app; a separate Terminal is preferable for controlled captures. Excluding the
collector does not remove its CPU cost or other effects on the system.

## Phase commands

The sampler runs in the background while you interact with either app. These
commands preserve the original manual workflow of labeling idle and work:

```text
idle prepared empty conversation
work same model and effort; prompt submitted
note final output has 35 numbered paragraphs; output bytes recorded separately
post response visible and Working ended
status
stop capture complete
```

Commands mark your actions; `work` does not run a model. Repeated phase names
remain separate phases. `status` shows app, descendants/extras, recognized Codex
backend subset, and the full RSS total. The backend subset is already contained
in descendants/extras; do not add it to the total again.

- `fail REASON` records a product failure, including a crash, timeout, or incorrect
  output. Failed attempts remain available for reporting and comparison.
- `invalid REASON` records a harness failure, such as a missed automation action.
- `done [NOTE]` / `stop [NOTE]` finishes a measurement without claiming workload verification.
- `quit`, EOF, or Ctrl+C preserves an incomplete attempt.
- `footprint` / `snap` takes an optional diagnostic snapshot of the currently observed
  group. This can perturb the app. Its interval and membership checks are logged;
  it is not included as a sampled peak or silently substituted for RSS.

Unlike v3, `idle` and `post` do not automatically take intrusive footprint
snapshots; use `snap` explicitly. The original interface keeps its one-second
default cadence, as does the newer `record` interface. Use `--interval 0.2` for
denser sampling after checking collector cost; maxima still remain sampled maxima.

A brief noninteractive capture is also available:

```sh
just memory-bench record --target smaller-talk --pid 12345 \
  --label collector-check --duration 3 --phase idle --context busy
```

The outcome is `measurement_only`, never automatically a passing workload.
The command exits nonzero for product failures, invalid/incomplete attempts, or
captures without successful samples. Missing initial PIDs fail before creating
a capture. Stopping the collector does not stop the target processes.

## Evidence and reports

Each attempt gets a new directory under `.dev/memory-bench/`, or the new directory
specified by `--output`. Existing captures cannot be overwritten. Artifacts are:

- `manifest.json`: pinned root identities, root binary hashes when readable,
  bundle versions, host/system memory context, framework revision and dirty state,
  framework source hashes, requested interval, embedded workload manifest and
  hash, and explicit outcome.
- `events.jsonl`: append-only phase markers, timestamped per-process samples,
  collection durations, identities entering/leaving scope, errors, and notes.
- `report.json` and `report.md`: per-phase RSS distributions, sampling gaps,
  failures, and measurement limits. Reports can be regenerated without changing
  the raw trace, including after interruption.
- `totals.csv`, `processes.csv`, `summary.json`, and `metadata.json`: familiar
  v3-style exports, regenerated at finish or with `report`. Export format version
  4 preserves the main RSS/CPU columns and adds phase identities and validity
  fields. Missing-root totals are blank, not zero. Per-sample swap is unavailable;
  start/end VM/swap counters are in the manifest. The legacy summary aggregates
  repeated phase names; use `report.json` for separate phase sequences.
- Optional `footprint-*.json` / `.txt`: raw macOS diagnostic output. A successful
  stable-membership group snapshot supports deduplicated footprint inspection;
  denied, timed-out, or changing-membership captures are marked unusable.

Executable paths are recorded, but process arguments and environment variables
are not. Notes, workload content, paths, and diagnostics may contain information
you enter; review them before sharing. The root hash identifies the app binary,
not its loaded libraries or all backend binaries. Record backend versions in the
workload manifest for controlled runs. The framework revision is not the target
app's build revision.

```sh
just memory-bench report .dev/memory-bench/RUN
just memory-bench compare .dev/memory-bench/SMALLER_RUN .dev/memory-bench/CODEX_RUN
```

Comparison uses **full process totals on both sides**, joins phases by their
sequence number and name, retains failure outcomes and warnings, and warns about
missing/different workload manifests. It is descriptive: identical manifest hashes
do not prove execution equivalence. Do not compare runs with different preceding
phases or cherry-pick successful attempts.

Reports also show the explicitly labeled subset of samples with a recognized
Codex child present. This makes execution overhead visible when a manual work
marker includes a long idle tail. Backend presence includes setup and shutdown;
it does not establish model-generation latency.

The supplied real-Codex workload is a starting template. Copy it and fill its
configuration placeholders before controlled runs. Use identical model, effort,
workspace, instructions, tools, viewport, and starting conversation state; record
actual output size and completion time. Its prompt is the original v3 Rust
`merge_ranges` explanation with 35 numbered paragraphs and a test code block.
Real outputs differ, so deterministic
fixture workloads remain a separate implementation milestone.

## Accounting limits

RSS sums can double-count shared pages and omit compressed/swapped memory.
Maximum RSS means the largest observed sample, not the true allocation peak.
Missing/failed samples remain explicit and are never replaced with zero. A root
that exits remains pinned to its old identity; its absence is reported and those
samples are excluded from memory distributions. Determine normal exit versus
product failure from the workload evidence.

Identity uses PID plus `ps` start time (one-second resolution). Previously observed
children remain in scope after reparenting. Polling can miss short-lived processes
or a child that detaches between samples. The framework does not yet collect
process-start events or attribute remote execution, kernel services, shared GPU
resources, or work inside a shared backend to individual conversations.

The cadence uses monotonic deadlines rather than sleeping a full interval after
each collection. Raw timestamps expose actual gaps and sampler cost. Boundary
samples that straddle phase markers are retained but excluded from phase metrics.
Host load and start/end VM/swap counters give context; they cannot correct for
other work or turn busy-system results into controlled measurements.
CPU percentages from `ps` are approximate moving estimates, not measured CPU time
over each sample interval. They can exceed 100% across cores. UI/backend role
splits use executable paths rather than private command arguments and are heuristic.

## Validation and next implementation steps

```sh
just memory-bench-test
```

Tests exercise complete process totals, unrelated Codex exclusion, explicit extra
roots, reparenting, PID reuse, root exit, failed samples, interrupted traces, phase
boundaries, frozen manifests, and outcome preservation. A macOS probe briefly maps
and releases 16 MiB in a disposable Python process and checks child accounting and
exit. It does not assert stable RSS growth/recovery under system compression or
make application performance claims.

Initial framework validation on 2026-10-22 passed 28 tests, the original Codex
GUI diagnostic, and a disposable-process smoke check covering the original phase
commands, all eight report files, CPU/child accounting, descriptive comparison,
and a usable group footprint snapshot. The host was busy; these checks establish
collector behavior, not application performance. No live provider workload or
automated native UI scenario was run.

A subsequent [Smaller Talk-only release benchmark](../benchmarks/results/20261022T054648.450645Z-f5f468f6a230.json)
ran the original prompt through the native UI with real Codex and verified the
persisted response and child cleanup. That busy-system observation is separate
from initial collector validation and makes no regression claim. Reporting now
includes the Codex-child-present subset; subsequent shortcut checks bring the
framework suite to 31 tests.

The original command-driven benchmarker was found in
`Downloads/smaller_talk_resource_benchmark_v3.zip`. Its source, protocol, and prompt
are preserved byte-for-byte under `benchmarks/reference/v3/` with provenance
hashes. It already included Codex execution costs; the new framework preserves
that scope and adds pinned process identities, reparenting history, explicit
failure outcomes, collector exclusion, raw event timing, and descriptive reports.
The separate `.dev/record-memory.py` and the Downloads archive remain untouched.
Old CSVs are not automatically imported or reinterpreted as new controlled runs.

The automated extension adds recorded protocol playback, history/retention
scenarios, launch gating, frame evidence, real Codex coverage, per-process
footprint reads, independent launches, and paired prebuilt-binary comparisons.
The Python integrity suite now has 54 tests; the Rust workspace checks retain
their separate authenticated-provider exclusions. Quiet-system characterization,
agreed product budgets, wider workload coverage, and automated Instruments traces
remain future work. A passing scenario proves execution/capture integrity, not
that memory use is acceptable.

The [automated validation results](../benchmarks/results/20261022T164426.219401Z-d80dca840d32.json)
record the completed playback/live suite. The playback retention phases contain
zero Codex samples. After the final cycle, all samples contain only the app;
median app footprint rises from approximately 94 to 523 MiB. Some earlier cycles
include Git helpers, which are counted separately. This is an app-retention
observation on a busy host, not an assertion that expected CLI peaks are defects.
Full narrative reports remain local under ignored `docs/benchmarks/`.

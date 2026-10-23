# Smaller Talk vs Codex Desktop — disposable resource benchmark

This compares **the total running local process groups** (UI, helpers, Codex app-server, and descendants). It evaluates the current chat-only POC, not full tool execution or browser integration. It does not make a formal performance acceptance requirement.

## Prepare once

1. Use the same Mac, power connection, display setup, Codex account, model, and reasoning setting for both runs. Quit the other measured app before each run. Stop other active agent jobs, background builds, downloads, and other variable workloads. Prefer the same window size and fresh chat in each app.
2. Build Smaller Talk **in release mode** for a fair comparison with the production Codex Desktop app:

   ```sh
   cd /path/to/smaller_talk
   SQLX_OFFLINE=true cargo build --locked --release -p smaller-talk-desktop
   export SMALLER_TALK_BIN="$PWD/target/release/smaller-talk-desktop"
   export SMALLER_TALK_DATA_DIR="$(mktemp -d /tmp/smaller-talk-bench.XXXXXX)"
   "$SMALLER_TALK_BIN" db-create
   "$SMALLER_TALK_BIN" db-migrate
   ```

3. Keep that Terminal open as your **app launch Terminal**. Start the monitor in another Terminal first (instructions below). Then return to the launch Terminal and run:

   ```sh
   cd /path/to/benchmark-workspace
   "$SMALLER_TALK_BIN"
   ```

   The exported `SMALLER_TALK_DATA_DIR` remains set in this Terminal. The working directory matters: Smaller Talk uses the launch directory as Codex's `cwd`. In Codex Desktop, open that same workspace/project in a fresh chat.

4. Copy `smaller_talk_workload_prompt.txt` into the composer for both runs. It asks Codex **not to use tools** because Smaller Talk v0 deliberately has no approval UI and declines tool approvals. Do not benchmark repository edits yet. Keep settings and prompts identical. Network/model response timing can still vary.

## Run the monitor for each application separately

In one Terminal, before launching the measured application:

```sh
python3 smaller_talk_benchmark.py --mode smaller --out ./benchmark-runs/smaller-01
```

Use another Terminal to launch Smaller Talk. In the **monitor's** Terminal, enter these commands as the run proceeds:

| Command | When | Purpose |
| --- | --- | --- |
| `idle` | App launched, window loaded, and settled for ~30 seconds | Begins idle sampling; records a de-duplicated macOS `footprint` snapshot |
| `work` | Immediately before submitting the benchmark prompt | Begins active-work sampling |
| `post` | When the agent response completes | Begins post-work sampling; records another `footprint` snapshot |
| `stop` | After ~30 more seconds, with the window still open | Saves summary and ends measurement |

For Codex Desktop, **quit Smaller Talk and its active agent process first**, then in a fresh monitor Terminal:

```sh
python3 smaller_talk_benchmark.py --mode codex --out ./benchmark-runs/codex-01
```

Launch Codex Desktop from the Dock, or open its verified application bundle (for example, `open /Applications/ChatGPT.app` on installs where its bundle identifier is `com.openai.codex`), use a fresh chat in the same folder, and repeat the identical steps. Only one measured app should be open at a time. You can type `snap` during work for an extra physical-memory snapshot, but the snapshot itself briefly adds system overhead; it is optional.

**Important:** Do not include build, database setup, or `cargo run` in the sampled periods. Run the already-built Smaller Talk executable. If automatic main-process detection fails, you can pass `--pid <MAIN_PID>`; the monitor will still gather its children. Check the monitor's first `Detected app main process` line to confirm it selected the right root.

## Results

Each output directory contains:

- `totals.csv`: one-second summed process RSS, approximate CPU percent, process counts, system load, system-wide swap, and heuristic UI/backend split.
- `processes.csv`: corresponding per-process breakdown, without full command-line arguments.
- `footprint/*`: macOS physical-memory reports for idle and post-work. The `footprint` tool **deduplicates shared mapped pages between supplied PIDs**, making these snapshots more appropriate than simply adding Electron helpers' RSS.
- `summary.json`: per-phase median/peak summed RSS, mean/peak CPU, count, and the timestamps of the phase markers.
- `metadata.json`: machine and sample configuration.

Start with **idle footprint**, **idle CPU**, **active CPU**, **active summed-RSS peak**, and **post-work footprint**. Summed RSS is a convenient live proxy but **double-counts shared pages**; it is not the same as app memory in Activity Monitor. `%CPU` from `ps` is a moving estimate and can exceed 100% across cores. The backend/UI split is approximate. System-wide swap/load are environmental context, not app-attributed usage. The monitor does not record per-app watts, GPU resource use, or disk/network I/O. Power measurements would need a separate instrumented test.

Repeat at least twice (ideally alternate A/B order) before drawing strong conclusions. Keep each run's output in a distinct directory. Provide both run folders or zip them to review the results together.

## Safety and limitations

- The monitor **does not** launch, stop, kill, modify, or inject into Codex or Smaller Talk. It only reads process information and creates benchmark logs under the supplied output directory.
- Codex Desktop is expected to bundle its own Codex app-server; Smaller Talk starts the installed Codex executable. Both are counted as part of their respective process groups.
- The process grouping selects the app's main PID and its descendants; Codex Desktop bundle helpers are additionally included by path under the **same bundle with the verified `com.openai.codex` identifier**. Child processes of those helpers are also included. Review the per-process log if a helper is reparented or unexpectedly excluded. Kernel/WindowServer GPU work cannot be reliably attributed to individual apps by this script.
- One-second sampling may miss very short-lived subprocesses. `footprint` can fail if macOS permissions restrict inspection; the raw failure is saved and continuous RSS/CPU logging continues.

## Process-detection check (benchmark v3)

If the script shows zero processes, **do not treat 0 MiB as a result**. Open Codex Desktop, then run in a separate terminal:

```sh
python3 smaller_talk_benchmark.py --mode codex --diagnose
```

For the confirmed `/Applications/ChatGPT.app` installation, the bundle identifier was verified as `com.openai.codex` using:

```sh
plutil -extract CFBundleIdentifier raw -o - /Applications/ChatGPT.app/Contents/Info.plist
```

The diagnostic should now display **one verified root**, not each app-server/helper subprocess separately.

The scan finds only processes whose **application bundle identifier** is `com.openai.codex` and whose executable matches that bundle’s declared main executable. Newer Codex Desktop builds may appear as `/Applications/ChatGPT.app/Contents/MacOS/ChatGPT`. `pgrep -x Codex` is **not** a reliable GUI PID finder; it can return many Codex backend/helper processes. If bundle verification is unavailable, inspect the app root with `ps -axww -o pid,ppid,command | grep -F ".app/Contents/MacOS/"` and verify its `Contents/Info.plist` `CFBundleIdentifier` is `com.openai.codex` before providing `--pid <MAIN_PID>`:

```sh
python3 smaller_talk_benchmark.py --mode codex --pid 12345 --out runs/codex-02
```

Replace `12345` with the running GUI PID, not an app-server or command-line Codex process. The script warns after 15 seconds without a verified process, refuses to pick arbitrarily among multiple app roots, and exits unsuccessfully if none was captured. Ensure each phase has detected processes before relying on the CSV output.

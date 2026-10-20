# Smaller Talk

For the current chat workspace, upgrade instructions, and verification, see [Chat workspace](docs/chat-workspace.md) and [Settings, Git review, and response display](docs/settings-git-response-display.md). The phase reports below describe the earlier proof of concept.

Smaller Talk is a macOS GPUI/Rust proof of concept for local, app-owned Codex conversations. It keeps chat identity and messages in SQLite and uses the local Codex app-server for execution. This report records the phase implementation, review corrections, verification evidence, and limits before the phase Review checkpoint.

## Local setup

With [just](https://github.com/casey/just) installed, run `just` to list the available commands. Use `just db-setup` before first launch, then `just run` or `just dev` to restart the app on changes. The latter requires `cargo install cargo-watch --locked`. Use `just format`, `just lint`, `just check`, and `just test` for workspace checks, and `just build` for an optimized release in `target/release/`. `just lint` treats warnings as errors, including the existing Clippy findings documented below.

The database lives at `~/Library/Application Support/SmallerTalk/chats.sqlite3`. Setup is explicit and separate from opening the app:

```sh
cargo run -p smaller-talk-desktop -- db-create
cargo run -p smaller-talk-desktop -- db-migrate
cargo run -p smaller-talk-desktop
```

Set `SMALLER_TALK_DATA_DIR` to choose another directory, including a temporary directory for local evaluation. `db-create` creates the SQLite file; `db-migrate` applies the versioned schema. Ordinary startup does not create or migrate the database. SQLite has no PostgreSQL role/bootstrap machinery, while the two local setup responsibilities remain separate.

The SQLx query macros validate external SQL files against the schema. Checked query metadata is committed under `.sqlx`, so normal builds do not require a live development database. To refresh that metadata after changing SQL or migrations, use SQLx CLI 0.8.6 with the local database selected:

```sh
DATABASE_URL=sqlite:///absolute/path/to/chats.sqlite3 cargo sqlx prepare --workspace
```

If SQLx CLI is not installed with SQLite support, install the matching CLI with `cargo install sqlx-cli --version 0.8.6 --no-default-features --features sqlite,rustls`.

## Workspace

- `smaller-talk-domain` owns app chat, message, and opaque execution-binding values.
- `smaller-talk-application` owns chat use cases, message persistence ports, and the provider-neutral execution capability.
- `smaller-talk-database` implements those ports with SQLite and feature-local checked SQL.
- `smaller-talk-infrastructure` implements the application execution capability with the Codex app-server process adapter.
- `smaller-talk-desktop` resolves launch configuration, composes the services, renders basic Markdown, and presents the GPUI window.

The launch working directory supplies the Codex workspace context. There is no project picker or project catalog. Codex protocol types and process handling stay inside the Infrastructure adapter; the application service receives app-owned requests and text/binding events. The Desktop entry resolves the executable from `SMALLER_TALK_CODEX_BIN`, `PATH`, or the supported Homebrew/local/NVM locations and passes an explicit path into the adapter. `ConversationService::send_message` takes a named `SendMessageInput`; adapter errors preserve their opaque source for diagnostics. GPUI is pinned to 0.2.2 and uses its runtime Metal shader compilation option: rendering stays on Apple's Metal path, while shader compilation occurs at application startup instead of requiring the separate `metal` compiler during Cargo builds.

## Phase 02 behavior and evidence

The table connects each binding requirement in this phase to implementation evidence and the result observed in this checkout. `cargo test --workspace` and a desktop build were run with the committed SQLx metadata (`SQLX_OFFLINE=true`). The live check used an authenticated local Codex CLI and a temporary SQLite data directory.

| Requirement | Evidence and result |
| --- | --- |
| R-01, R-02: native Rust/GPUI window independent of Electron and T3 | The desktop crate is the GPUI/Rust entry point and its Cargo dependency graph contains no Electron or T3 runtime. `SQLX_OFFLINE=true cargo build --locked -p smaller-talk-desktop` succeeded, and the built checkout opened as a native macOS window. This confirms the exercised checkout build and launch, not a packaged or fresh installation. |
| R-03: playbook boundaries adapted to SQLite | The workspace separates Domain, Application, Database, Infrastructure, and Desktop. The Codex process/protocol dependencies are in Infrastructure; Desktop resolves environment configuration and injects the executable path. `ConversationService::send_message` uses named `SendMessageInput`, and `ExecutionError` retains an opaque source. Feature-local SQLite access uses SQLx checked queries. `SQLX_OFFLINE=true cargo test --workspace` and the locked desktop build passed; no PostgreSQL service is required. |
| R-04: durable app-owned chats, messages, and provider binding | The SQLite store persists messages and the opaque Codex thread binding separately from Codex history. In a live prompt run, SQLite contained the user and assistant messages and `adapter_key=codex` with the native thread ID. After quitting and reopening the app, the stored transcript loaded. |
| R-05: create and select app-created chats | The GPUI chat list creates and selects local chats. A live UI check created a second temporary chat, observed its empty transcript, and selected the original to see its own saved messages. The second chat was then deleted; the evaluation database contained only the original app-created chat. The list was not populated from Codex history. |
| R-06: delete local chat content | The chat-list Delete action was exercised on the temporary evaluation chat. It disappeared from the UI and SQLite; no native Codex deletion is claimed. The integration tests also cover chat deletion behavior. |
| R-07: Codex behind an app-owned execution boundary | `ConversationService` depends on `ExecutionAdapter`, not Codex protocol types. The Infrastructure adapter starts the local `codex app-server --stdio` process and maps JSON-RPC responses, thread binding, and assistant deltas into application-owned types. Adapter mapping, binding reuse, subprocess failure sources, and malformed JSON sources are covered by Infrastructure tests; a live prompt succeeded using the installed CLI. No Claude adapter is included. |
| R-08: send a prompt and display streamed Codex output | In the real Codex UI run, a prompt produced a visible progressive response and the completed user/assistant pair was saved in SQLite. For the review correction, a controlled app-server fixture accepted an uppercase `AB` prompt. Switching to another chat and returning before completion showed the accumulated `PREFIXMIDDLE` while the turn remained marked Working; `TAIL` then appeared on completion. The active-transcript unit test also merges the live text over a transcript reload. |
| R-09: basic Markdown and fenced code | The real Codex response rendered prose and a fenced Rust code block, and the formatted transcript reloaded after restart. For the correction, a controlled fixture streamed nested bold/italic text and inline code; the UI displayed all three styles while Working, then showed following plain text without leaked emphasis. After normal quit/reopen, the same saved response retained those styles. Parser tests reparse successive stream prefixes and assert ordered, nonoverlapping UTF-8-safe ranges. This remains a limited subset, not full Markdown. |
| R-10: resume the same native thread after normal restart | After quitting and reopening the app, a follow-up prompt was sent from the existing chat. SQLite retained the same native Codex thread ID across all four prompt/response pairs, and the app displayed the continued response. This verifies normal restart continuation for the exercised app-created thread. |
| R-11: no project organizer | The chat workflow ran from the launch working directory without a project picker or project catalog. Project organization remains outside this POC. |

## Review corrections and targeted results

The correction checks used the Cargo-built executable from this existing macOS checkout in a temporary `.app` wrapper, with a separate temporary SQLite directory and a local deterministic app-server fixture. The fixture was not a Codex installation and the correction-specific stream/Markdown results do not extend the earlier real Codex protocol check. In the UI, Shift+A and Shift+B entered uppercase text in the prompt; the fixture received and displayed the resulting `AB` prompt. The two-chat stream test switched away and back before completion and visibly retained the partial response. The Markdown fixture exposed nested emphasis and inline code while Working, then added plain trailing text; the final transcript showed the trailing text unstyled and kept the formatting after application restart. A desktop unit test independently exercises the stream switch, intermediate transcript reload, remaining delta, and completion sequence.

The Playbook boundary corrections are checked in the manifests and source: `smaller-talk-infrastructure` owns Codex process/protocol dependencies and adapter implementation, while Desktop reads executable configuration and injects a `PathBuf`. Application callers pass `SendMessageInput` by name. Tests confirm `ExecutionError` preserves an opaque I/O source and the adapter preserves subprocess and JSON parse sources. The review's optional strict Clippy run reported a test-only `await_holding_lock` diagnostic despite the explicit `drop`; no binding lint gate was supplied, so this optional failure is recorded and does not affect the required workspace tests or build.

## Verification conditions and limits

The real Codex checks ran on macOS ARM64 with Rust 1.94.0 from the existing source checkout, using an ad hoc temporary `.app` wrapper around the Cargo-built executable and `SMALLER_TALK_DATA_DIR=/tmp/smaller-talk-phase02-eval`. The local Codex CLI was authenticated; the installed CLI reported version 0.160.0. The adapter uses that local CLI's app-server protocol. Correction checks used the same existing-checkout launch arrangement with a separate temporary state directory and a deterministic local fixture. No clean install, release-package launch, second machine, fresh authentication setup, or other Codex installation/version was exercised. The documentation and generated protocol types informed the adapter, but only the installed local protocol was exercised end to end.

The app-server may request approval for provider actions. Since v0 has no approval UI, this adapter declines those requests; a prompt requiring such an approval is unsupported and was not tested. The live prompts explicitly required no tools. Error handling for normal process/protocol failures is implemented, but broad recovery and crash reconciliation are outside this phase. App messages and Codex-native state are not one transaction: a crash during thread creation can still leave a native orphan for manual cleanup, as allowed by the contract. A provider failure can leave the already-persisted user prompt without a completed assistant message.

A short phase-local RSS observation was recorded with `ps` at 0.2-second intervals while the evaluation app and its Codex child ran on this Mac. After a 30-line response was loaded and the app was idle, the GPUI process was 100,512 KiB (about 98 MiB); no app-spawned Codex child remained. During a subsequent 60-line response stream, sampled GPUI RSS was about 110–113 MiB, the Node CLI launcher was about 47 MiB, and its Codex app-server child reached about 235 MiB, about 393 MiB combined. These are single-run process RSS observations, not a benchmark or a numerical acceptance threshold; the host had other applications and Codex processes running, which were excluded by matching the POC app's child process. They do not establish RAM savings versus another desktop client.

The checks for this phase do not measure long-session behavior, interruption/retry UX, crash recovery, accessibility, fresh installation, or performance across workload sizes. This Work correction submission returns to the phase Review checkpoint. The accepted scope ends with the Codex chat shell; Claude/provider switching, imported history, project organization, rename/search, advanced Markdown, annotations, turn-management controls, display-only response rewriting, and guaranteed Codex-native deletion remain deferred.

## Composer and streaming review corrections

All five findings were confirmed against the source. These corrections retain the existing package boundaries and POC scope.

| Finding | Correction and evidence |
| --- | --- |
| Append-only composer | The composer is now a local GPUI input entity adapted from GPUI 0.2.2's `examples/input.rs`, using `EntityInputHandler` and `ElementInputHandler` for native text entry. It supports grapheme-aware left/right movement and deletion, up/down movement, keyboard and mouse selection, clipboard actions, a 100-edit undo/redo history, and multiline input. Its four-line viewport follows the caret vertically and horizontally; it does not implement a general editor or soft wrapping. Enter submits and Shift+Enter inserts a newline. The adapted source retains its Apache-2.0 license notice and license file. |
| Streaming scroll jumps | Snapshot application compares the selected chat and its visible messages. Changed text follows the bottom only if the previous layout was already within two pixels of the bottom. Background-chat changes leave the visible transcript and offset alone. Explicit selection changes and prompt submission still move to the bottom. Transcript reloads use the same rule, so a delayed reload cannot unconditionally override a reading position. |
| stderr pipe closes at 16 KiB | `AsyncReadExt::take` previously ended reading and dropped the pipe at the cap. A close-after-limit subprocess exercise with 1 MiB of stderr failed (exit 120, no completion marker). The replacement drains to EOF while retaining at most 16 KiB, including for invalid or split UTF-8. A regression test runs a real noisy subprocess and asserts successful exit, its stdout completion marker, and bounded diagnostic retention. |
| Rust 1.85 declaration | The Markdown let-chain requires 1.88, but the locked `uuid` 1.28.0 dependency declares 1.89. The workspace now declares Rust 1.89; a locked workspace check on the installed 1.89.0 toolchain passed on macOS ARM64. Downgrading syntax alone would not have made the locked workspace compatible with 1.85. |
| Provider names in application errors | Execution, task, and empty-response errors now refer to the agent. Opaque adapter diagnostics remain intact, and the Codex adapter keeps its provider-specific diagnostic details. No additional provider or abstraction was added. |

For this correction, `SQLX_OFFLINE=true cargo test --workspace --locked` passed all 15 tests; `SQLX_OFFLINE=true cargo build --locked -p smaller-talk-desktop`, `cargo fmt --all -- --check`, and `git diff --check` passed. `SQLX_OFFLINE=true CARGO_TARGET_DIR=/tmp/smaller-talk-msrv-target cargo +1.89.0 check --workspace --locked` passed. Tests and the native build used Rust 1.94.0. Optional strict Clippy remains blocked by pre-existing `collapsible_if`, `match_single_binding`, and test-only `await_holding_lock` findings. Clippy passed with only those three existing lint categories allowed; these allowances were command-line verification settings, not committed lint suppressions.

The practical exercise used the current Cargo-built native macOS app in a temporary `.app` wrapper, a separate SQLite directory (`/tmp/smaller-talk-review-input-scroll`), and a deterministic local app-server fixture. The UI visibly verified insertion inside existing text, selection replacement, undo restoring text and selection, redo, copy/cut/paste, Unicode paste, mouse drag selection and click positioning, grapheme-safe deletion of a combining sequence and an emoji, Shift+Enter, and Enter submission. The fixture received the edited two-line prompt exactly as displayed, including `é🙂`. It wrote 1 MiB of stderr before streaming 90 numbered paragraphs. While the selected chat streamed, scrolling up left saved paragraphs 06–09 at the same position across subsequent deltas. Selecting the other chat and scrolling up likewise kept paragraphs 10–12 fixed while the first chat continued. Returning to the responding chat showed the accumulated response and resumed bottom following as further paragraphs arrived.

This exercise used a fixture, not authenticated Codex. No new real-provider, fresh-install, or other-platform result is claimed. Native IME composition, dictation, and accessibility were not practically exercised; ordinary Unicode paste and grapheme editing were exercised. The existing POC scope remains unchanged.

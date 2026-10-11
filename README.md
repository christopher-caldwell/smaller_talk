# Smaller Talk

Smaller Talk is a macOS GPUI/Rust proof of concept for local, app-owned Codex conversations. It keeps chat identity and messages in SQLite and uses the local Codex app-server for execution. This checkout implements the selected v0 chat loop and is submitted for phase review; this report records the evidence and limits of the Work gate.

## Local setup

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
- `smaller-talk-desktop` composes the services, runs the Codex app-server adapter, renders basic Markdown, and presents the GPUI window.

The launch working directory supplies the Codex workspace context. There is no project picker or project catalog. Codex protocol types and process handling stay inside the desktop adapter; the application service receives app-owned requests and text/binding events. GPUI is pinned to 0.2.2 and uses its runtime Metal shader compilation option: rendering stays on Apple's Metal path, while shader compilation occurs at application startup instead of requiring the separate `metal` compiler during Cargo builds.

## Phase 02 behavior and evidence

The table connects each binding requirement in this phase to implementation evidence and the result observed in this checkout. `cargo test --workspace` and a desktop build were run with the committed SQLx metadata (`SQLX_OFFLINE=true`). The live check used an authenticated local Codex CLI and a temporary SQLite data directory.

| Requirement | Evidence and result |
| --- | --- |
| R-01, R-02: native Rust/GPUI window independent of Electron and T3 | The desktop crate is the GPUI/Rust entry point and its Cargo dependency graph contains no Electron or T3 runtime. `SQLX_OFFLINE=true cargo build -p smaller-talk-desktop` succeeded, and the built checkout opened as a native macOS window. This confirms the exercised checkout build and launch, not a packaged or fresh installation. |
| R-03: playbook boundaries adapted to SQLite | The workspace separates Domain, Application, Database, and Desktop/Infrastructure; application services use capability ports, and feature-local SQLite access uses SQLx checked queries. `SQLX_OFFLINE=true cargo test --workspace` and SQLx metadata preparation succeeded. No PostgreSQL service is required. |
| R-04: durable app-owned chats, messages, and provider binding | The SQLite store persists messages and the opaque Codex thread binding separately from Codex history. In a live prompt run, SQLite contained the user and assistant messages and `adapter_key=codex` with the native thread ID. After quitting and reopening the app, the stored transcript loaded. |
| R-05: create and select app-created chats | The GPUI chat list creates and selects local chats. A live UI check created a second temporary chat, observed its empty transcript, and selected the original to see its own saved messages. The second chat was then deleted; the evaluation database contained only the original app-created chat. The list was not populated from Codex history. |
| R-06: delete local chat content | The chat-list Delete action was exercised on the temporary evaluation chat. It disappeared from the UI and SQLite; no native Codex deletion is claimed. The integration tests also cover chat deletion behavior. |
| R-07: Codex behind an app-owned execution boundary | `ConversationService` depends on `ExecutionAdapter`, not Codex protocol types. The desktop adapter starts the local `codex app-server --stdio` process and maps JSON-RPC responses, thread binding, and assistant deltas into application-owned types. Adapter mapping and binding reuse are covered by desktop/service tests; a live prompt succeeded using the installed CLI. No Claude adapter is included. |
| R-08: send a prompt and display streamed Codex output | In the live UI, a prompt produced the requested response, the composer showed a working state, and a longer response appeared progressively before completion. The completed user/assistant pair was saved in SQLite. A failed or interrupted turn's partial assistant text is not committed as a completed message. |
| R-09: basic Markdown and fenced code | The live first response rendered prose and a fenced Rust code block in the UI; the completed transcript reloaded with the formatting after restart. Parser unit tests cover basic Markdown blocks and inline formatting. This is a limited presentation subset, not full Markdown. |
| R-10: resume the same native thread after normal restart | After quitting and reopening the app, a follow-up prompt was sent from the existing chat. SQLite retained the same native Codex thread ID across all four prompt/response pairs, and the app displayed the continued response. This verifies normal restart continuation for the exercised app-created thread. |
| R-11: no project organizer | The chat workflow ran from the launch working directory without a project picker or project catalog. Project organization remains outside this POC. |

## Verification conditions and limits

The live checks ran on this Mac from the existing source checkout, using an ad hoc temporary `.app` wrapper around the Cargo-built executable and `SMALLER_TALK_DATA_DIR=/tmp/smaller-talk-phase02-eval`. The local Codex CLI was authenticated; the installed CLI reported version 0.160.0. The adapter uses that local CLI's app-server protocol. No clean install, release-package launch, second machine, fresh authentication setup, or other Codex installation/version was exercised. The documentation and generated protocol types informed the adapter, but only the installed local protocol was exercised end to end.

The app-server may request approval for provider actions. Since v0 has no approval UI, this adapter declines those requests; a prompt requiring such an approval is unsupported and was not tested. The live prompts explicitly required no tools. Error handling for normal process/protocol failures is implemented, but broad recovery and crash reconciliation are outside this phase. App messages and Codex-native state are not one transaction: a crash during thread creation can still leave a native orphan for manual cleanup, as allowed by the contract. A provider failure can leave the already-persisted user prompt without a completed assistant message.

A short phase-local RSS observation was recorded with `ps` at 0.2-second intervals while the evaluation app and its Codex child ran on this Mac. After a 30-line response was loaded and the app was idle, the GPUI process was 100,512 KiB (about 98 MiB); no app-spawned Codex child remained. During a subsequent 60-line response stream, sampled GPUI RSS was about 110–113 MiB, the Node CLI launcher was about 47 MiB, and its Codex app-server child reached about 235 MiB, about 393 MiB combined. These are single-run process RSS observations, not a benchmark or a numerical acceptance threshold; the host had other applications and Codex processes running, which were excluded by matching the POC app's child process. They do not establish RAM savings versus another desktop client.

The checks for this phase do not measure long-session behavior, interruption/retry UX, crash recovery, accessibility, or performance across workload sizes. The phase Review remains to assess the implementation and these results. The accepted scope ends with the Codex chat shell; Claude/provider switching, imported history, project organization, rename/search, advanced Markdown, annotations, turn-management controls, display-only response rewriting, and guaranteed Codex-native deletion remain deferred.

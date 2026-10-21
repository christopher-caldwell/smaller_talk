# Chat workspace

Smaller Talk remains a native Rust/GPUI chat app. This expansion adds conversation tabs, projects, search, Codex/Claude controls, attachments, Markdown, activity feedback, and native Plan mode. The earlier phase reports in the README describe the original proof of concept.

## Run or upgrade

For an existing database, quit the old app, run `just db-migrate`, then `just run`. For a new installation, run `just db-setup` before `just run`. Ordinary startup never provisions or migrates the database. The migration preserves existing chats, messages, and Codex bindings.

The app uses the installed Codex and Claude Code CLIs and their existing sign-ins. Set `SMALLER_TALK_CODEX_BIN` or `SMALLER_TALK_CLAUDE_BIN` if discovery cannot find an executable. `SMALLER_TALK_DATA_DIR` selects a separate data directory. The default remains `~/Library/Application Support/SmallerTalk`.

## Use

Model and effort pickers open above their composer controls. Use Up/Down, Home/End, and Enter to choose, or Escape to close and restore focus. Tab/Shift+Tab reaches existing buttons and inputs; Enter/Space activates buttons, with visible focus borders. Effort choices mark the current selection and show the discovered default when available. Tooltips explain provider, mode, attachment, tab, and navigation controls. Search has a clear button; Escape clears search and returns to the composer. Chats with an empty or “Untitled” saved title display a first-message preview without changing their stored title.

GPUI 0.2.2 does not expose a native accessibility-tree API. These keyboard and visual improvements do not provide full VoiceOver semantics; native control roles, values, and message-tree exposure remain unresolved within this GPUI version.

- Selecting a sidebar chat opens or focuses its tab. Closing a tab keeps the chat and draft. Tabs, the active tab, drafts, selections, reading positions, and sidebar state survive restart.
- New Chat, Cmd+T, or Cmd+N opens one reusable unsent draft. The conversation enters saved history only when its first message or attachment is sent; whitespace alone does not create a chat. The draft, selected settings, and attachments survive switching tabs or restarting. Closing its tab retains the draft for the next New Chat action. Cmd+W closes the current tab; Ctrl+Tab and Ctrl+Shift+Tab switch tabs. Cmd+B toggles the sidebar and Cmd+F focuses search. The sidebar folds when resizing into a compact window. Expanding it at narrow widths overlays navigation rather than shrinking the conversation; selecting a chat closes the overlay.
- Project menus create, rename, or remove projects. Chat menus assign or unassign chats. A project can have an optional local working folder; removing it keeps its chats. Projects without folders use the launch directory.
- Search matches literal text in titles, messages, and project names, including Unicode lowercase matching. It returns at most 50 chats with excerpts. Selecting a message result reveals that message. Matching runs in the database adapter over streamed rows, keeping complete histories out of the search UI.
- The composer’s model menu includes both providers. Effort choices come from the selected model’s advertised capabilities. Each chat has independent settings, execution, errors, and draft context.
- Chat/Plan selects native provider behavior. Questions and approve/deny requests appear inline. A request to leave Plan mode requires an explicit choice.
- Drag files onto the composer or use its attachment button. PNG, JPEG, GIF, and WebP images become native visual input. Other files become saved paths the agent can inspect using its own tools. Attachment-only messages work.
- Enter sends; Shift+Enter inserts a newline. Text wraps and the composer grows within a bounded viewport. Responses support tables, lists, task lists, emphasis, strikethrough, quotes, links, and highlighted code. Select text and use Cmd+C, or use a response/code Copy button.

## Context and storage

Switching provider or effective working folder creates a fresh native session containing the saved visible conversation and attachment references. Subsequent turns resume that session. Hidden reasoning and provider-specific tool state are not transferred. Failed retries reuse the persisted user message; partial responses remain saved. Oversized or missing context fails visibly rather than being silently omitted.

Attachments are copied into app-managed storage; source files remain untouched. A message accepts at most 100 files, 10 MiB per image, 80 MiB total images, and 50 MiB per other file. Transferred image context counts toward the same limits. Provider limits still apply. Thumbnails have bounded dimensions and decoding allocation; their GPU caches belong to mounted thumbnail elements. Unreferenced app-owned files are collected at startup.

SQLite stores projects, settings, drafts, attachment references, turn settings/status, and restored UI state. Startup marks incomplete turns as interrupted. Transcript rows use GPUI’s variable-height list, with cached measurements and bounded Markdown/text-view caches. Closing an inactive tab clears its heavy view caches; closing a responding tab keeps its execution alive and clears those caches when it finishes.

## Verification

Run:

```sh
just format-check
just lint
just check
just test
SQLX_OFFLINE=true cargo build --workspace --locked
SQLX_OFFLINE=true rustup run 1.89.0 cargo check --workspace --all-targets --locked
```

The authenticated smoke test is deliberately opt-in and uses only synthetic text and the committed blue-square image:

```sh
SMALLER_TALK_LIVE=1 SQLX_OFFLINE=true cargo test -p smaller-talk-desktop \
  --test live_providers -- --ignored --nocapture
```

Checked SQLx query descriptions are committed under `.sqlx`. Regenerate them against a migrated development database with a SQLite-enabled SQLx CLI. If the installed CLI lacks the SQLite driver, SQLx macros also support `SQLX_OFFLINE=false SQLX_OFFLINE_DIR="$PWD/.sqlx" DATABASE_URL=sqlite:///absolute/development.sqlite3 cargo check --workspace --all-targets`; ensure the database crate is rebuilt when refreshing descriptions. No live build database is needed for the normal offline commands above.

### Native and provider evidence, 2026-10-19

A disposable native `.app` bundle and separate SQLite directory exercised tables/code/nested Markdown, fast scrolling, message search/reveal, selection/copy, wrapped Unicode drafts, project creation with a folder, image picking/thumbnails, tab restoration, compact layout, and synthetic question/approval round trips. During a synthetic background response, the selected history tab retained its reading position. Protocol fixtures cover both providers, settings, images, questions, approvals, stream deduplication, and authoritative completed plans. Automated persistence tests cover upgrades, projects, bounded/literal search, restart, provider/folder handoffs, failed retries, and simultaneous independent chats.

The authenticated test passed Codex → Claude → Codex using GPT-6 Luna and Claude Haiku 5.5, native Plan mode on both providers, and transferred visual context. All three responses retained the synthetic token and identified the image as blue; the temporary working folder remained empty. Native checks used Codex CLI 0.163.0 and Claude Code 2.1.303. Codex collaboration mode requires its experimental app-server capability; model lists are discovered rather than copied from screenshots.

A local debug-build RSS comparison used the original repository HEAD and the expanded app with the same synthetic history and streaming fixture. The original app peaked at 127,776 KiB native RSS (145,152 KiB including its fixture child). The expanded app peaked at 130,320 KiB (146,240 KiB combined), with other tabs retained. These are single-run observations, not a performance guarantee or an acceptance threshold. Clearing caches need not immediately return allocator pages to macOS.

The OS file picker and native image import were exercised directly; the external-file drop handler uses GPUI’s native `ExternalPaths` interface. Broad accessibility, native IME/dictation, and other platforms were not exhaustively exercised. Browsers, embedded editors, and terminals remain outside this chat-focused expansion. Read-only Git review is now available; see [Settings, Git review, and response display](settings-git-response-display.md).

### Compact composer and scrolling polish

The composer uses three, five, or seven visible draft rows according to window height, with a stable two-row toolbar in narrow panes. Attachment chips stay in one horizontally scrollable strip. Long drafts support wheel scrolling with a small position indicator; edits and caret navigation reveal the caret again. Single-line fields scroll horizontally instead of wrapping. Conversation widths come from measured pane layout and retain reading position through resizing. Vertical gestures over tables/code continue scrolling the transcript; focused wide blocks support Left/Right and Home/End for horizontal navigation.

At short window heights, an inline pending request temporarily takes input priority while the unsent draft stays saved. Each new request starts with its own scroll position, so approval headings remain visible after scrolling a preceding question.

Native verification used a disposable database at the 480-logical-pixel minimum width and short height, a 40-line Unicode draft, an image attachment, long history, narrow tables, and synthetic question/approval responses. Wheel scrolling, caret reveal, compact sidebar overlay, search/reveal, reading-position preservation across resizing, keyboard horizontal scrolling, vertical scrolling through wide content, and request scroll reset were exercised. The native automation API delivered zero horizontal wheel delta, so horizontal scrolling was verified by keyboard rather than claiming a trackpad gesture test. No real-provider turns were needed for this presentation-only pass.

### Palette and composer references

The interface now uses a deep-purple navigation surface, charcoal conversation/composer surfaces, neutral gray outlines and focus borders, and a green active-chat highlight. Purple stays in navigation; the chat header remains charcoal. Navigation structure is unchanged. New chat has a compose icon and highlights on hover or keyboard focus rather than a permanent fill. The composer keeps attachment and Chat/Plan controls on the left, with adjacent model/effort controls and a circular send action on the right. Provider details remain in the model tooltip; microphone and access-policy controls were not added.

Chat actions and project choices use compact row-anchored menus, icon labels, and a separated destructive action. Rename and deletion confirmation stay explicit dialogs. The skill research is documented in [design options](skills-design-options.md); no skill interface or invocation behavior was added in this pass.

### Durable follow-ups

Message queuing and verified native steering are implemented. See [message queue behavior, recovery, and verification](message-queue.md). Enter and Cmd+Enter send or queue; Shift+Enter remains newline. Pending requests keep a compact composer available for queued follow-ups.

### Settings, Git review, and answer shortening

Global provider/model settings, model search, structured approvals, aligned activity, read-only Git review, and opt-in response shortening are documented in [the feature guide](settings-git-response-display.md). Upgrade existing databases with `just db-migrate` before starting this build.

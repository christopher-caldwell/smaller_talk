# Chat workspace

Smaller Talk remains a native Rust/GPUI chat app. This expansion adds conversation tabs, projects, search, Codex/Claude controls, attachments, Markdown, activity feedback, and native Plan mode. The earlier phase reports in the README describe the original proof of concept.

## Run or upgrade

For an existing database, quit the old app, run `just db-migrate`, then `just run`. For a new installation, run `just db-setup` before `just run`. Ordinary startup never provisions or migrates the database. The migration preserves existing chats, messages, and Codex bindings.

The app uses the installed Codex and Claude Code CLIs and their existing sign-ins. Set `SMALLER_TALK_CODEX_BIN` or `SMALLER_TALK_CLAUDE_BIN` if discovery cannot find an executable. `SMALLER_TALK_DATA_DIR` selects a separate data directory. The default remains `~/Library/Application Support/SmallerTalk`.

## Use

Model and effort pickers open above their composer controls. Use Up/Down, Home/End, and Enter to choose, or Escape to close and restore focus. Tab/Shift+Tab reaches existing buttons and inputs; Enter/Space activates buttons, with visible focus borders. Effort choices mark the current selection and show the discovered default when available. Tooltips explain provider, mode, attachment, tab, and navigation controls. Search has a clear button; Escape clears search and returns to the composer. Chats with an empty or “Untitled” saved title display a first-message preview without changing their stored title.

Native checks of the upgraded app still expose window-level accessibility information without the app-drawn control roles, values, or message tree. Full VoiceOver semantics remain unverified.

- Selecting a sidebar chat opens or focuses its tab. Closing a tab keeps the chat and draft. Tabs, the active tab, drafts, selections, reading positions, and sidebar state survive restart.
- New Chat, Cmd+T, or Cmd+N opens one reusable unsent draft. The conversation enters saved history only when its first message or attachment is sent; whitespace alone does not create a chat. The draft, selected settings, and attachments survive switching tabs or restarting. Closing its tab retains the draft for the next New Chat action. Cmd+W closes the current tab; Ctrl+Tab and Ctrl+Shift+Tab switch tabs. Cmd+B toggles the sidebar and Cmd+F focuses search. The sidebar folds when resizing into a compact window. Expanding it at narrow widths overlays navigation rather than shrinking the conversation; selecting a chat closes the overlay.
- Use the + beside a project, its New chat menu item, or Start a chat in an empty project to open a draft assigned to that project. The draft appears under the project immediately; its first send saves the chat with that project and working folder. These actions reuse the unsent draft and preserve its text and attachments. Project menus also rename or remove projects. Chat menus assign or unassign chats. A project can have an optional local working folder; removing it keeps its chats. Projects without folders use the launch directory.
- Search matches literal text in titles, messages, and project names, including Unicode lowercase matching. It returns at most 50 chats with excerpts. Selecting a message result reveals that message. Matching runs in the database adapter over streamed rows, keeping complete histories out of the search UI.
- The composer’s model menu includes both providers. Effort choices come from the selected model’s advertised capabilities. Each chat has independent settings, execution, errors, and draft context.
- Chat/Plan selects native provider behavior. Questions and approve/deny requests appear inline. A request to leave Plan mode requires an explicit choice.
- Drag files onto the composer or use its attachment button. PNG, JPEG, GIF, and WebP images become native visual input. Other files become saved paths the agent can inspect using its own tools. Attachment-only messages work.
- Enter sends; Shift+Enter inserts a newline. Text wraps and the composer grows within a bounded viewport. Responses support tables, lists, task lists, emphasis, strikethrough, quotes, links, and highlighted code. Select text and use Cmd+C, or use a response/code Copy button.

## Input editing

GPUI Kit 0.8.0 supplies the unstyled `Textarea` for the composer and instructions editor, and `Input` for single-line fields. It owns macOS editing shortcuts, Unicode selection, clipboard, undo/redo, IME, wrapping, and scrolling. The app adapter owns submission, skill discovery and provider/workspace identity, attachments, and the existing durable draft format. Selected skills use native inline tokens; deletion and undo preserve their reference identity. See [GPUI Kit migration](gpui-kit-migration.md) for dependencies, local patches, and verification.

## Context and storage

Switching provider or effective working folder creates a fresh native session containing the saved visible conversation and attachment references. Subsequent turns resume that session. Hidden reasoning and provider-specific tool state are not transferred. Failed retries reuse the persisted user message; partial responses remain saved. Oversized or missing context fails visibly rather than being silently omitted.

Attachments are copied into app-managed storage; source files remain untouched. A message accepts at most 100 files, 10 MiB per image, 80 MiB total images, and 50 MiB per other file. Transferred image context counts toward the same limits. Provider limits still apply. Thumbnails have bounded dimensions and decoding allocation; their GPU caches belong to mounted thumbnail elements. Unreferenced app-owned files are collected at startup.

SQLite stores projects, settings, drafts, attachment references, turn settings/status, and restored UI state. Startup marks incomplete turns as interrupted. Transcript rows use GPUI’s variable-height list, with cached measurements and bounded Markdown/text-view caches. Closing an inactive tab clears its heavy view caches; closing a responding tab keeps its execution alive and clears those caches when it finishes.

## Split panes

Use the pane button above the conversation to split right or down, even out the layout, switch focus, or close a pane. Splits can be nested in either direction. Drag a divider to resize; double-click it to restore equal sizes. Each conversation keeps its own measured transcript width, reading position, composer, draft, model, and permissions.

An empty pane offers open conversations that are not already visible. Opening a conversation that another pane already shows focuses that pane. Closing a pane keeps its conversation open as a tab and does not stop its execution. The pane tree, divider ratios, visible conversations, and focused pane are saved locally with the existing session state and restored on restart.

| Shortcut | Action |
| --- | --- |
| Cmd+D | Split right |
| Cmd+Shift+D | Split down |
| Cmd+Option+] / Cmd+Option+[ | Focus next / previous pane |
| Cmd+Option+W | Close focused pane |

## Conversation permissions

The composer has a permission menu beside the model and effort controls. It identifies the selected provider and applies to that conversation. Each provider's choice is remembered separately, including after an application restart. Settings → Providers & Models also offers a permission default for each provider; those defaults apply to new conversations.

| Choice | Behavior |
| --- | --- |
| Provider default | Use the provider's native configuration. Existing conversations without a saved choice use this option. |
| Supervised | Request approval for commands and file changes. |
| Auto-accept edits | Allow file edits while other actions can require approval. |
| Auto | Use the provider's automatic action review. |
| Full access | Allow commands and edits without routine approvals. |

The adapters enforce these choices through native permission controls. Codex receives explicit approval, reviewer, and sandbox policies on thread start/resume and on each turn; Claude receives its permission-mode flag. Returning from an explicit mode to Provider default starts a fresh native session with visible conversation context so a resumed Codex thread cannot retain its earlier elevated policy. Native account, organization, and explicit ask/deny policies still apply. Unsupported controls surface provider errors; when Claude reports a different effective mode, the composer shows the requested and effective selections.

Chat/Plan remains a separate control. Permission changes are locked while an operation is running, and the selected choice is saved before another message can be sent. Response shortening keeps its separate isolated configuration.

## Verification

Run:

```sh
just format-check
just lint
just check
just test
SQLX_OFFLINE=true cargo build --workspace --locked
SQLX_OFFLINE=true rustup run 1.99.1 cargo check --workspace --all-targets --locked
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

## Project navigation and local archive (ST-013)

The controls beside **New chat** sort and filter projects. **Alphabetical** is the default. **Last chat updated** uses the latest saved user or assistant message in each project's unarchived chats; projects without messages sort last. Drafts and streaming tokens do not change the order. Chat ordering within a project stays the same.

**Filter projects** opens Project visibility, with search, individual switches, Show all, and Hide all. Hidden projects also disappear from sidebar search. The reset control beside the active filter shows all projects without changing the sort or collapse choices. Open tabs, panes, drafts, and execution continue normally. Newly created projects are visible, and the unassigned Chats group stays visible. Clicking a project's title or disclosure arrow collapses its children. Sorting, visibility, and collapse choices survive restart.

**Archive chat** is available in a saved chat's action menu once the chat is idle and its queue is empty. It saves a local archive timestamp and closes that chat's tabs and panes after the write succeeds. History, draft content, attachments, settings, and the native provider binding are retained. The app makes no provider archive or delete request. Existing Delete chat behavior remains separate.

The archive icon at the bottom of the sidebar, or in its collapsed rail, opens **Archived chats**. This view searches archived titles, project names, and saved messages across all projects, including hidden projects. Selecting a chat opens its transcript read-only. Restore returns it to ordinary chat navigation and opens it for continuation, preserving project filter and collapse choices. Startup removes stale archived chat references from saved tabs and panes.

## Commands and activity (ST-013)

Type `/` at the beginning of the composer to open the command picker. The initial command set contains only `/compact`; skills remain under `$name`. Arrows navigate, Enter accepts the suggestion without executing it, and Escape dismisses the picker. Submit `/compact` alone to compact the current native session. Attachments, arguments, mixed content, and unsupported command names are rejected without discarding the draft. Paths containing additional slashes, escaped commands, and commands inside prose or code remain ordinary text.

Compaction requires an idle, unarchived conversation with an empty queue and a binding valid for its current provider, folder, and custom instructions. Send a normal message first if the current configuration needs a new session. Codex resumes the thread and calls `thread/compact/start`; Claude resumes the session and sends its native `/compact` command. Native completion is required before reporting success. Claude can report that there is not enough history to compact. Compaction preserves the local transcript and binding and does not append a local user or assistant message.

Compaction runs through the same per-chat coordinator as delivery. Follow-up messages entered during compaction wait in the durable queue. A compaction failure holds them for review; there is no automatic command retry or text-prompt fallback. Startup has the existing 25-second deadline, followed by a 180-second compaction deadline. A timeout reports an uncertain outcome.

Starting, Working, Thinking, Responding, and Compacting display a small green pulse beside the status in the conversation and sidebar. Waiting for input remains a static attention indicator. The pulse uses a separate view, runs at most 20 frames per second, and respects reduced motion. Enabled send and queue controls use the activity green on hover and keyboard focus; disabled controls retain their muted appearance.

Migration `0007_chat_archive.sql` adds the archive field. Run `just db-migrate` before launching an existing installation; normal startup does not migrate the database. New navigation preferences default to alphabetical sorting, all projects visible, and all projects expanded when older session JSON is loaded.

The opt-in compaction smoke test uses disposable workspaces, local databases, and native sessions:

```sh
SMALLER_TALK_LIVE=1 cargo test -p smaller-talk-desktop --test compaction native_compaction_preserves -- --ignored --nocapture
```

It compacts both providers, verifies that local messages and bindings are unchanged, and asks each provider to recall a synthetic token on the next turn. This passed with Codex CLI 0.163.0 (`gpt-6-luna`) and Claude Code 2.1.303 (`claude-haiku-5-5`). Deterministic tests cover archive retention/failures, search filtering before its result limit, preference reopening, native compaction acceptance versus completion, unsupported/disconnected providers, timeout cleanup, and queue serialization.

Native UI verification used a disposable application, database, and deterministic Codex fixture. It covered wide and 480-logical-pixel windows, split panes, sorting, individual/bulk project visibility, filtered search, collapse and restart restoration, local archive/read-only viewing/restore, command selection versus execution, unsupported-command draft retention, compaction/working feedback, and green send/queue hover states. GPUI tests verify that the pulse stops after any outstanding frame when waiting or when reduced motion is enabled, and that stale archived tabs/panes are cleared on restoration. The ordinary application database was not migrated or changed by these checks.

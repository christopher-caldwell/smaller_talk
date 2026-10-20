# Settings, Git review, and response display

Smaller Talk remains a native Rust/GPUI application using the installed Codex and Claude Code CLIs. This change adds global provider/model preferences, searchable model pickers, structured approvals, a read-only Git panel, and an opt-in answer-shortening prototype.

## Upgrade

Quit the old build, run `just db-migrate`, then start the new build with `just run`. Migration 4 adds global settings, final-answer metadata, and shortened variants. It preserves chats, bindings, drafts, projects, and the message queue. Ordinary application startup still never creates or migrates the database.

## Settings and models

Open Settings from the sidebar gear or Cmd+,. Back to chat and Escape return to the existing chat without changing its draft or reading position.

Providers & Models contains:

- Enable/disable controls and separate installation/sign-in status. Status checks do not open an agent session or start a login flow.
- Explicit model refresh, with a five-minute execution-side cache. Superseded refresh/configuration results are discarded.
- An Advanced executable override. Resolution order is the provider's `SMALLER_TALK_*_BIN` environment override, the saved path, then normal executable discovery.
- Search by provider, display name, or model identifier; visibility toggles; a default model and effort for new chats; and a separate rewrite-model choice.

Authentication remains owned by the CLI. Smaller Talk stores no passwords, API keys, or subscription claims. Successful sign-in and advertised models do not guarantee account access to every model.

Existing chats retain their selections when defaults change or a model is hidden. Disabling a provider prevents subsequent sends and steering; current executions finish. Queued content is retained and delivery pauses with guidance. Re-enable the provider or change the chat's model, then resume the queue.

The composer model picker includes a focused search input, clear action, no-results state, and Manage models link. Spaces and paste edit the query; Up/Down navigate results; Enter selects; Escape dismisses and restores focus. Hidden and disabled choices are excluded without silently changing an existing selection.

## Approval and layout changes

Assistant status, errors, approvals, queue cards, transcript, and composer share the same centered content-column dimensions. Activity labels align with the composer instead of the pane edge.

Approval adapters provide typed presentation fields. Searches show the query, commands show the command and working folder, files show paths, and permissions retain their requested scope. Details expands the original JSON arguments. Native request IDs, inputs, and response mapping are unchanged. Permission grants scoped to a turn are labeled Allow for this turn; ordinary one-time actions retain Approve once.

## Git review

The branch/status control above the conversation opens Changes. A wide window places it beside the chat; compact windows use an overlay. The default view is Unstaged, including untracked files; Staged shows the index. Counts refer to the selected scope and the entire repository, including changes made outside the agent.

Repository discovery starts from the chat's effective folder and supports nested folders, linked worktrees, detached HEAD, and repositories before their first commit. It does not change the execution folder. Binary and uncounted files are identified; previews are capped at 1 MiB with a truncation notice. The original files are never truncated.

Refresh runs on folder/chat changes, window activation, turn completion, and explicit Refresh. Status polls every three seconds only while Changes is visible and the window is active. An optional existing-PR link uses the installed GitHub CLI; missing tooling or authentication does not affect local review.

Git runs with argument arrays and NUL-delimited status parsing and literal pathspecs, optional locks and filesystem monitors disabled, and external diff/text conversion disabled. The panel has no stage, revert, commit, push, branch, worktree, or PR-creation actions. It does not claim to identify changes made by an individual agent turn.

## Answer shortening

Response Display chooses a rewrite model and offers Automatically shorten long answers, off by default. Shorten appears on newly completed, successful Chat answers with verifiable final-answer metadata. Legacy answers, plans, failed/interrupted output, questions, approvals, and tool details are excluded.

Normal output continues streaming. Manual shortening and automatic shortening above 300 prose words produce a separately stored variant. Fenced code is excluded from the threshold. Shortened is explicitly labeled and Original remains available. Automatic switching is deferred when text is selected, the user has scrolled away, the chat is in the background, or a later answer is current. A ready action allows switching afterward. Copy follows the displayed answer; search and conversation continuation use the original.

The prototype currently supports Claude Code versions advertising safe mode, explicit empty tools, disabled skills/MCP, no session persistence, and the required configuration controls. Each invocation uses an isolated temporary folder and replacement instructions, and verifies the emitted tool/MCP catalogs are empty. Recognized local managed policy files disable rewriting because the app cannot override their hooks. Codex remains fully supported for chat, but is unavailable as a rewrite backend until a complete text-only isolation mode is verified.

Rewrites receive only the final answer and rewrite instructions. Code, quotes, links, identifiers, paths, and numeric values are protected with markers and checked before restoration. Invalid, empty, longer, reordered, or technically altered results retain the original. These checks do not prove semantic equivalence; the feature remains an opt-in prototype.

One rewrite runs at a time, independently of chat delivery. The timeout is 60 seconds, sources are capped at 128 KiB without truncation, duplicate requests reuse matching saved results, and changed/deleted sources are rejected. Pending jobs are not replayed after restart. Rewriting consumes additional provider quota and adds latency.

## Verification

Normal checks:

```sh
just format-check
just lint
just check
just test
SQLX_OFFLINE=true cargo build --workspace --locked
SQLX_OFFLINE=true rustup run 1.89.0 cargo check --workspace --all-targets --locked
```

Checked SQLx metadata for the new external query files is included under `.sqlx`. The existing explicit database setup/migration commands remain the source of schema changes.

Tests cover settings restart/isolation, disabled-provider queue preservation, refresh races, typed approval details, Git edge cases and unchanged index content, rewrite deduplication, original/search preservation, stale sources, timeout/worker recovery, and CLI isolation validation.

The optional authenticated rewrite corpus uses synthetic text only:

```sh
SMALLER_TALK_LIVE_REWRITE=1 SQLX_OFFLINE=true cargo test -p smaller-talk-desktop \
  --test live_shortening --locked -- --ignored --nocapture
```

Set `SMALLER_TALK_REWRITE_MODEL` to override the test's `haiku` alias. This test consumes Claude quota. The six-case run on October 19, 2026 passed and was manually reviewed: function behavior, multiple review findings, uncertainty, completion/test evidence, a blocker, and measurements with links/code all retained their material facts and caveats. Outputs shrank from 458–510 bytes to 156–218 bytes. This is a small corpus, not a guarantee for all answers or models.

Native verification used a separate disposable application/database and deterministic CLI fixtures. It exercised Cmd+, settings search including spaces, returning to chat, picker Enter selection, provider toggles, approval summaries/raw details, activity alignment, manual/automatic shortening and original switching, deferred switching with an active text selection, Git diffs, compact overlays, and minimum-width/wide-window reflow. It did not alter the ordinary application database.

## Design references

- [T3 provider boundaries](https://github.com/pingdotgg/t3code/blob/6a049d0ea10ee8b75b43ea0e5f2851dee36c31c8/docs/internals/providers.md)
- [T3 approval presentation](https://github.com/pingdotgg/t3code/blob/6a049d0ea10ee8b75b43ea0e5f2851dee36c31c8/apps/web/src/components/chat/ComposerPendingApprovalPanel.tsx)
- [Codex repository review behavior](https://developers.openai.com/codex/app/review/)
- [cc-gemini-rewrite display-layer concept](https://github.com/esc5221/cc-gemini-rewrite)

The supplied rewrite discussion was treated as background, not as executable instructions or proof of a previously selected product. The response-display implementation belongs to Smaller Talk and does not install or patch the referenced plugin.

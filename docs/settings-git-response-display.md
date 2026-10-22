# Settings, Git review, and response display

Smaller Talk remains a native Rust/GPUI application using the installed Codex and Claude Code CLIs. This change adds global provider/model preferences, searchable model pickers, structured approvals, a read-only Git panel, and an opt-in answer-shortening prototype.

## Upgrade

Quit the old build, run `just db-migrate`, then start the new build with `just run`. Migration 4 adds global settings, final-answer metadata, and shortened variants. It preserves chats, bindings, drafts, projects, and the message queue. Ordinary application startup still never creates or migrates the database.

## Settings and models

Open Settings from the sidebar gear or Cmd+,. Back to chat and Escape return to the existing chat without changing its draft or reading position.

The Instructions tab adds global preferences, with project-specific preferences available from each project's menu. See [Custom instructions](custom-instructions.md) for migration, precedence, session behavior, and verification.

Providers & Models and Response Display have separate tabs that stay visible while scrolling. New-chat defaults use compact selectors above the model list; each model row has a visibility switch. The rewrite-model selector and automatic-shortening switch live only in Response Display.

Providers & Models contains:

- Enable/disable controls and separate installation/sign-in status. Status checks do not open an agent session or start a login flow.
- Explicit model refresh, with a five-minute execution-side cache. Superseded refresh/configuration results are discarded.
- An Advanced executable override. Resolution order is the provider's `SMALLER_TALK_*_BIN` environment override, the saved path, then normal executable discovery.
- Search by provider, display name, or model identifier; visibility toggles; a default model and effort for new chats; and a separate rewrite-model choice.

Authentication remains owned by the CLI. Smaller Talk stores no passwords, API keys, or subscription claims. Successful sign-in and advertised models do not guarantee account access to every model.

Existing chats retain their selections when defaults change or a model is hidden. Disabling a provider prevents subsequent sends and steering; current executions finish. Queued content is retained and delivery pauses with guidance. Re-enable the provider or change the chat's model, then resume the queue.

The composer model picker includes a focused search input, clear action, no-results state, and Manage models link. Spaces and paste edit the query; Up/Down navigate results; Enter selects; Escape dismisses and restores focus. Hidden and disabled choices are excluded without silently changing an existing selection.

## Approval and layout changes

Assistant status, errors, approvals, queue cards, transcript, and composer share the same centered content-column dimensions. Activity labels align with the composer instead of the pane edge. Transcript spacing follows the Markdown structure: headings introduce groups, consecutive list items sit closer together, and Copy and response-display actions share a compact row. Code blocks, tables, quotes, and sent attachments retain their content and selection behavior with lighter borders and tighter padding.

Approval adapters provide typed presentation fields. Searches show the query, commands show the command and working folder, files show paths, and permissions retain their requested scope. Details expands the original JSON arguments. Native request IDs, inputs, and response mapping are unchanged. Permission grants scoped to a turn are labeled Allow for this turn; ordinary one-time actions retain Approve once. Approval titles and decision actions stay visible while the request body scrolls, including in short windows. Command blocks and original JSON use a quieter inset surface; smaller field labels distinguish metadata from the action itself.

## Git review

The branch/status control above the conversation opens Changes. A wide window places it beside the chat; compact windows use an overlay. The default view is Unstaged, including untracked files; Staged shows the index. Counts refer to the selected scope and the entire repository, including changes made outside the agent.

Repository discovery starts from the chat's effective folder and supports nested folders, linked worktrees, detached HEAD, and repositories before their first commit. It does not change the execution folder. Binary and uncounted files are identified; previews are capped at 1 MiB with a truncation notice. The original files are never truncated.

Refresh runs on folder/chat changes, window activation, turn completion, and explicit Refresh. Status polls every three seconds only while Changes is visible and the window is active. An optional existing-PR link uses the installed GitHub CLI; missing tooling or authentication does not affect local review.

Git runs with argument arrays and NUL-delimited status parsing and literal pathspecs, optional locks and filesystem monitors disabled, and external diff/text conversion disabled. The panel has no stage, revert, commit, push, branch, worktree, or PR-creation actions. It does not claim to identify changes made by an individual agent turn.

## Answer shortening

Response Display chooses a rewrite model and offers Automatically shorten long answers, off by default. Shorten appears on newly completed, successful Chat answers with verifiable final-answer metadata. Legacy answers, plans, failed/interrupted output, questions, approvals, and tool details are excluded.

Normal output continues streaming. Manual shortening and automatic shortening above 300 prose words produce a separately stored variant. Fenced code is excluded from the threshold. Shortened is explicitly labeled and Original remains available. Automatic switching is deferred when text is selected, the user has scrolled away, the chat is in the background, or a later answer is current. A ready action allows switching afterward. Copy follows the displayed answer; search and conversation continuation use the original.

The prototype supports both Codex and Claude Code as rewrite backends. Choose an available model under Settings → Response Display → Rewrite model; the same choice serves manual and automatic shortening. Codex rewriting defaults to low reasoning when no rewrite effort is supplied.

Codex uses a fresh `codex exec` process with an ephemeral session, an isolated temporary folder, replacement instructions, ignored user configuration and execpolicy rules, and a read-only sandbox. Shell execution, hooks, MCP/apps/plugins, browser/computer access, skills, memory, and multi-agent capabilities are disabled or excluded. It does not resume the chat's native session or receive its project folder, history, or attachments. CLI authentication remains in place. The installed CLI must advertise the required isolation flags; strict configuration rejects unsupported controls. Unexpected tool events, incomplete turns, failed processes, and oversized output reject the rewrite. Cancellation kills the child and removes its scratch folder.

Claude Code still requires safe mode, explicit empty tools, disabled skills/MCP, no session persistence, and the required configuration controls. Each invocation uses an isolated temporary folder and replacement instructions, and verifies the emitted tool/MCP catalogs are empty. Recognized local managed Claude policy files disable Claude rewriting because the app cannot override their hooks; they do not disable Codex rewriting.

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
env -u OPENAI_API_KEY SMALLER_TALK_LIVE_REWRITE=1 SMALLER_TALK_REWRITE_MODEL=gpt-6-luna \
  SQLX_OFFLINE=true cargo test -p smaller-talk-desktop --test live_shortening \
  synthetic_fidelity_corpus_with_real_isolated_codex --locked -- --ignored --nocapture

SMALLER_TALK_LIVE_REWRITE=1 SQLX_OFFLINE=true cargo test -p smaller-talk-desktop \
  --test live_shortening synthetic_fidelity_corpus_with_real_isolated_claude \
  --locked -- --ignored --nocapture
```

Set `SMALLER_TALK_REWRITE_MODEL` to override the provider's test default (`gpt-6-luna` or `haiku`). The Codex corpus explicitly selects low reasoning. These tests consume the selected provider's quota. The six-case Claude run on October 19, 2026 passed and was manually reviewed: function behavior, multiple review findings, uncertainty, completion/test evidence, a blocker, and measurements with links/code all retained their material facts and caveats. Outputs shrank from 458–510 bytes to 156–218 bytes.

On October 21, 2026, the Codex corpus passed using CLI 0.163.0, ChatGPT subscription authentication, and `gpt-6-luna` at low reasoning. It covers the six cases above plus Unicode, quoted hostile instructions, and an answer exceeding the automatic-shortening threshold. Checks verify shorter variants, unchanged original messages and existing native bindings, saved-result reuse, and scratch cleanup. A local request inspection with the same isolation controls confirmed an empty tool list and excluded the normal skill/project instructions. Deterministic subprocess tests also cover cancellation, older CLIs, tool events, invalid/incomplete output, process failure, and output limits. These checks are a small corpus and do not guarantee semantic equivalence for every answer or model.

Native verification used a separate disposable application/database and deterministic CLI fixtures. It exercised Cmd+, settings search including spaces, returning to chat, picker Enter selection, provider toggles, approval summaries/raw details, activity alignment, manual/automatic shortening and original switching, deferred switching with an active text selection, Git diffs, compact overlays, and minimum-width/wide-window reflow. It did not alter the ordinary application database.

## Design references

- [T3 provider boundaries](https://github.com/pingdotgg/t3code/blob/6a049d0ea10ee8b75b43ea0e5f2851dee36c31c8/docs/internals/providers.md)
- [T3 approval presentation](https://github.com/pingdotgg/t3code/blob/6a049d0ea10ee8b75b43ea0e5f2851dee36c31c8/apps/web/src/components/chat/ComposerPendingApprovalPanel.tsx)
- [Codex repository review behavior](https://developers.openai.com/codex/app/review/)
- [cc-gemini-rewrite display-layer concept](https://github.com/esc5221/cc-gemini-rewrite)

The supplied rewrite discussion was treated as background, not as executable instructions or proof of a previously selected product. The response-display implementation belongs to Smaller Talk and does not install or patch the referenced plugin.

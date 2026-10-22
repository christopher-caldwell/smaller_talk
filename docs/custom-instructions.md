# Custom instructions

Global instructions live in **Settings → Instructions**. A project's menu opens **Instructions…** for that project. The existing settings tabs and project metadata editor keep their current structure. Slack's native preferences supplied the visual reference: clear section titles, adjacent helper text, generous spacing, subtle dividers, and a selective preview.

Each scope has an enable checkbox, multiline editor, explicit Save and Cancel actions, and a combined preview. Enter adds a line; Cmd+S saves. Disabling instructions preserves their text. Drafts survive switching between instruction scopes during the current application session. Instructions are limited to 16 KiB per scope, with no null characters. Project text supports projects with or without a repository folder.

Quit the old application, run `just db-migrate`, then launch the updated build. Migration 5 adds project instructions, per-turn snapshots, and a native-session instruction fingerprint. Startup does not migrate the database. Global instructions use the existing settings document with backward-compatible defaults; project instructions use their project UUID and are deleted with that project.

## Execution behavior

The application composes enabled global and project text before each new turn. The wrapper asks the assistant to prefer project instructions over conflicting global preferences, while following the user's current explicit request and preserving Chat/Plan mode and tool permissions. Provider and repository instruction files can also apply. Model adherence is not guaranteed.

Instructions are separate from the user prompt and saved transcript:

- Codex receives `developerInstructions` on thread start/resume. Its collaboration-mode `developer_instructions` remains null so native Chat/Plan behavior is retained.
- Claude Code receives `--append-system-prompt`, retaining its normal system prompt.
- Answer shortening uses its existing isolated rewrite prompt and does not receive custom instructions.

The exact composed snapshot is committed with the turn before provider execution. New user messages and attachment links are recorded in that same database transaction. A persistence failure prevents provider execution. The **Instructions** action beneath a sent user message opens its read-only snapshot, including instructions for an accepted steering message's parent turn. Older turns are identified as having no historical instruction snapshot.

Queued messages resolve the latest saved instructions when dispatched. An active response and its accepted steering retain the original snapshot. Retrying a failed turn uses that turn's original snapshot, including after later preference edits.

A fingerprint of the effective text controls native-session reuse. Unchanged text reuses the session. An effective edit, enabling instructions, or clearing them starts a new native session with the existing saved-conversation handoff. Visible history and attachment references transfer; hidden reasoning and native tool state do not. This avoids stale appended instructions surviving a resume. Editing disabled text does not reset a session. There is no repository-file scanning or provider configuration rewriting.

## Verification

Workspace tests cover durable scope isolation, project deletion, text bounds, instruction changes and clearing, native-session reuse, retry snapshots, queued delivery versus steering, and rollback before execution. Provider protocol fixtures verify the native instruction fields and preserved collaboration mode.

The optional authenticated test uses synthetic greetings and an empty disposable workspace:

```sh
SMALLER_TALK_LIVE=1 SQLX_OFFLINE=true cargo test -p smaller-talk-desktop \
  --test custom_instructions \
  authenticated_instructions_resume_edit_clear_and_plan_both_providers \
  --locked -- --ignored --nocapture
```

On October 21, 2026, this passed with Codex CLI 0.163.0 (`gpt-6-luna`) and Claude Code 2.1.303 (`haiku`). Both followed global/project markers in Plan mode, retained them on resume, used updated project text in Chat mode, and stopped emitting the markers after both scopes were disabled. Session identities confirmed reuse before edits and replacement after edits/clearing. The test workspace remained empty.

Native UI verification used a separate application bundle and disposable database. It covered multiline editing, Unicode, Cmd+S, canceling edits, project navigation, combined previews, a real completed chat turn, and its historical snapshot at wide and compact sizes. Slack preferences were inspected without changing their settings. The ordinary Smaller Talk database was not touched.

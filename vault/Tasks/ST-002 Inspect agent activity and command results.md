---
id: ST-002
project: Smaller Talk
issue_type: Story
area: Activity
candidate: Proposed
status: Backlog
priority:
rank:
owner:
target:
size:
created: 2026-10-22
source: "[[Sources/2026-10-22 Cursor comparison]]"
tags: [backlog]
---

# ST-002 — Inspect agent activity and command results

## Source Code

- `crates/application/src/lib.rs` — execution and conversation events
- `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`
- `crates/desktop/src/shell.rs`, `crates/desktop/src/transcript.rs`
- `crates/database/src/`

## Description

As a user, I want to inspect what an agent actually did without asking it to narrate every step. Current broad activity labels lose file, search, command, and result details. Cursor's compact activity summary expanded into file names, read ranges, edits, and test output.

## Outcome

Each turn offers a quiet, expandable history of observable agent actions and their results directly in the conversation.

## Acceptance Criteria

- [ ] Show compact live/completed activity summaries for provider events actually available to the app.
- [ ] Expanding an entry reveals relevant file/range or search information, and command, folder, result, exit status, and output when supplied.
- [ ] Running, completed, failed, and interrupted actions remain distinguishable after reload.
- [ ] Associate entries with the correct chat and turn, including background execution and accepted steering.
- [ ] Cap persisted output and preview sizes; mark truncation and unavailable details explicitly.
- [ ] Opening or updating details preserves reading position, selection, and composer focus; approval controls retain their existing meaning.

## Scope

### In

- Provider event mapping, durable activity records, bounded summaries, and expandable details.

### Out

- A general interactive terminal, inferred hidden reasoning, or treating model prose as verified execution evidence.

## Implementation Notes

- Store bounded records on disk and load detailed output on demand. Do not retain every expanded transcript or raw protocol frame in memory.
- Some protocol events may need correlation across start, delta, and completion boundaries; preserve stable identity and ordering.

## Dependencies / Blockers

- No prerequisite ticket identified. Provider event coverage needs a capability audit.
- Supplies evidence useful to [[ST-003 Show work and verification summaries in chat]]; the first UI increment need not require that story.

## Questions

- Which action types deserve separate rows versus a grouped exploration summary?
- What output size and retention limits are appropriate for long sessions?

## Validation

- [ ] Use fixtures for both providers, failures, duplicate/completion events, concurrent chats, restart, and large output.
- [ ] Inspect command evidence from a synthetic live task and verify copy/expand/scroll behavior.
- [ ] Measure retained memory over repeated expansion, collapse, tab close, and history reload.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-003 Show work and verification summaries in chat]]

## Notes

This is evidence about observable actions, not a requirement to expose private model reasoning.

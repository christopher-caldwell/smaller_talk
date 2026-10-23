---
id: ST-010
project: Smaller Talk
issue_type: Spike
area: Files
candidate: Deferred exploration
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

# ST-010 — Evaluate an embedded file editor

## Source Code

- `crates/desktop/src/rich_text.rs`, `crates/desktop/src/composer.rs`
- `crates/desktop/src/shell/repository.rs`, `crates/desktop/src/shell.rs`
- `docs/settings-git-response-display.md`, `docs/chat-workspace.md`

## Description

An embedded editor was explicitly deferred. The user's infrequent use of Changes suggests first clarifying whether they need a quick file peek, a way to reference a selection, a small manual correction, or a full editing workflow.

## Outcome

A decision record selects an appropriate boundary between in-chat file inspection, external-editor handoff, and possible embedded editing.

## Acceptance Criteria

- [ ] Identify concrete file workflows and distinguish viewing, selecting context, reviewing differences, and editing.
- [ ] Compare bounded read-only peeks and external-editor handoff with a minimal embedded editor.
- [ ] Document editing responsibilities if proposed: unsaved state, external file changes, save errors, encoding, large/binary files, keyboard input, and accessibility.
- [ ] Assess lifecycle and memory implications, distinguishing measured prototype data from estimates.
- [ ] Produce a build/defer recommendation and tightly scoped follow-up candidates without making Changes the required entry point.

## Scope

### In

- User workflow refinement, option comparison, and an optional isolated feasibility prototype if selected during planning.

### Out

- Shipping an IDE, replacing the composer, or assuming the existing text-selection component is already a file editor.

## Implementation Notes

- A file-reference picker does not require an editor. Preserve that independence when evaluating [[ST-004 Reference workspace files and line selections]].
- Existing link handling and read-only previews may support a smaller first increment.

## Dependencies / Blockers

- No prerequisite ticket identified. Agree the representative workflow and investigation budget when selected.

## Questions

- What file task currently forces an unwanted app switch?
- Would inspect-and-reference satisfy the need without adding manual editing?

## Validation

- [ ] Demonstrate the selected workflows with in-chat/external options and record remaining gaps.
- [ ] If prototyped, check repeated open/close memory and external-change/save behavior before recommending adoption.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-003 Show work and verification summaries in chat]]
- [[ST-004 Reference workspace files and line selections]]

## Notes

Deferred exploration candidate; no editor implementation has been selected.

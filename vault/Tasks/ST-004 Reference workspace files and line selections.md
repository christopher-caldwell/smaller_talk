---
id: ST-004
project: Smaller Talk
issue_type: Story
area: Composer
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

# ST-004 — Reference workspace files and line selections

## Source Code

- `crates/desktop/src/composer.rs`, `crates/desktop/src/shell/composer_panel.rs`
- `crates/desktop/src/shell/skills.rs`, `crates/desktop/src/shell/drafts.rs`
- `crates/domain/src/workspace.rs`, `crates/application/src/lib.rs`
- `crates/infrastructure/src/attachments.rs`

## Description

As a user, I want to select a workspace file or a relevant range as context without manually copying paths or importing every file as an attachment. Cursor's context picker made choosing the fixture README straightforward. Smaller Talk already has attachments and exact-source skill references, which remain distinct concepts.

## Outcome

Searchable file references and line selections can accompany a prompt and survive draft, queue, and retry workflows.

## Acceptance Criteria

- [ ] A keyboard-accessible picker searches the effective workspace and inserts an identifiable file reference.
- [ ] A reference can identify a bounded line range, with clear file and range information before sending.
- [ ] Define and disclose whether dispatch uses current file contents, a saved snapshot, or a path-only reference; never imply omitted content was sent.
- [ ] Missing, moved, oversized, binary, or out-of-scope files produce clear feedback while preserving the draft.
- [ ] Draft switching, undo/redo, restart, queue editing, dispatch, retries, and provider handoffs preserve reference identity and the chosen content semantics.
- [ ] Search and previews remain bounded for large repositories and do not collide with existing skill or ordinary text editing behavior.

## Scope

### In

- Workspace file lookup, reference/range presentation, persistence, and provider delivery semantics.

### Out

- A full editor, mandatory whole-repository indexing, or replacing existing image/file attachments and skill references.

## Implementation Notes

- Treat reference semantics as a design decision before transport changes. A file path, attachment copy, and content snapshot have different freshness and retry behavior.
- Reuse the effective-workspace rules used for sending and skill discovery.

## Dependencies / Blockers

- No prerequisite ticket identified. An embedded editor is not required for choosing files or entering ranges.

## Questions

- How should users choose a range when no editor is open?
- What should happen to saved references when the chat's project folder changes?

## Validation

- [ ] Exercise Unicode/spaced paths, symlinks, workspace changes, file edits/deletion before dispatch, and bounded searches.
- [ ] Verify initial send, steering, queue, restart, retry, and handoff for both providers.
- [ ] Measure picker and reference retention on a large repository.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-007 Inspect effective conversation context]]

## Notes

Do not infer that a selected path means the provider has read that file.

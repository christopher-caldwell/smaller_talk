---
id: ST-006
project: Smaller Talk
issue_type: Story
area: Conversations
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

# ST-006 — Fork a conversation from a message

## Source Code

- `crates/desktop/src/transcript.rs`, `crates/desktop/src/shell/navigation.rs`
- `crates/application/src/lib.rs`, `crates/application/src/workspace.rs`
- `crates/domain/src/lib.rs`, `crates/domain/src/workspace.rs`
- `crates/database/src/`
- `docs/chat-workspace.md`

## Description

As a user, I want to try an alternative approach from an earlier point without changing the original discussion or manually copying its context. Cursor's Fork chat control created a separate conversation in the comparison.

## Outcome

A new app-owned conversation begins from a selected boundary and can continue independently, with clear lineage and shared-workspace behavior.

## Acceptance Criteria

- [ ] Fork from a defined message boundary into a clearly identified new chat, preserving the original transcript.
- [ ] Copy only the intended visible context and relevant delivered references/settings; drafts, unsent queues, and later messages are not silently included.
- [ ] The fork has independent execution identity and never accidentally resumes the original native thread as a second writer.
- [ ] Preserve attachment availability and historical instruction metadata under an explicit ownership policy, including after deleting either chat.
- [ ] Show source-chat lineage and make it clear when both conversations share the same working folder and files.
- [ ] Forking does not automatically start a provider turn; unsupported or oversized handoffs fail visibly and consistently.

## Scope

### In

- App-owned transcript branching, lineage, context transfer, and independent execution state.

### Out

- Implicit Git branch/worktree creation, cloning hidden reasoning/tool state, or automatically running parallel implementations.

## Implementation Notes

- Reuse established visible-context handoff rules only where they satisfy fork semantics; do not assume provider-native fork parity.
- File isolation is a separate product decision. Conversation independence alone does not isolate file edits.

## Dependencies / Blockers

- No prerequisite ticket identified. Context/reference ownership needs refinement before persistence design.

## Questions

- Should the default fork copy current settings or settings from the selected turn?
- How should source lineage behave if the original chat is renamed or deleted?

## Validation

- [ ] Fork at multiple boundaries, including failed/interrupted turns and chats with attachments, skills, and pending queues.
- [ ] Verify independent continuation for both providers, restart, and deletion of either side.
- [ ] Measure duplication and memory use for long transcripts and several unopened forks.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-007 Inspect effective conversation context]]

## Notes

Candidate only; creating this ticket does not authorize new agent tasks or workspace branches.

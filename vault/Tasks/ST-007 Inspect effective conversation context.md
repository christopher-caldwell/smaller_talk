---
id: ST-007
project: Smaller Talk
issue_type: Story
area: Context
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

# ST-007 — Inspect effective conversation context

## Source Code

- `crates/desktop/src/shell/instructions.rs`, `crates/desktop/src/shell/skills.rs`
- `crates/application/src/instructions.rs`, `crates/application/src/workspace.rs`
- `crates/application/src/lib.rs`, `crates/domain/src/workspace.rs`
- `docs/custom-instructions.md`, `docs/chat-workspace.md`

## Description

As a user, I want to know which folder, app instructions, skills, and file references apply to a conversation, especially after changing projects or providers. Smaller Talk already has historical instruction snapshots and explicit provider/folder handoffs; their combined context is not presented in one concise place.

## Outcome

An optional inspector explains app-controlled context for the next dispatch and the recorded context of a selected turn.

## Acceptance Criteria

- [ ] Show the effective working folder and whether it comes from a project or launch fallback.
- [ ] Distinguish next-dispatch settings from a running or historical turn's recorded settings and instructions.
- [ ] Identify selected skills, attachments, and available file references, including missing or unresolved items.
- [ ] Explain when a provider/folder/instruction change will create a fresh native session using visible conversation history.
- [ ] State the limits of inspection: provider-managed instructions, hidden reasoning, and native tool state are not necessarily visible or transferred.
- [ ] Inspection does not start a turn, alter settings, or load every attachment/transcript into memory.

## Scope

### In

- A concise combined view of existing app-owned context and session-handoff information.

### Out

- Claiming to enumerate the provider's full hidden prompt, editing provider configuration, or a token-usage meter.

## Implementation Notes

- Reuse the effective-workspace calculation and saved instruction snapshots rather than reconstructing historical state from current preferences.
- Label unknown or legacy context explicitly.

## Dependencies / Blockers

- No prerequisite ticket identified. [[ST-004 Reference workspace files and line selections]] can later extend the inspector; it need not block existing-context visibility.

## Questions

- Which facts belong in a compact summary versus expanded details?
- How should paused queued messages display context that will be resolved only at dispatch?

## Validation

- [ ] Verify project fallback, provider/folder changes, instruction edits, queued delivery, retries, steering, and legacy turns.
- [ ] Confirm read-only behavior and bounded loading on a large chat with many references.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-004 Reference workspace files and line selections]]
- [[ST-008 Display provider-reported context usage]]

## Notes

Existing per-message Instructions snapshots are a foundation to extend, not a missing capability to recreate.

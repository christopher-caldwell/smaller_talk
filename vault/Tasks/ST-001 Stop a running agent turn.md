---
id: ST-001
project: Smaller Talk
issue_type: Story
area: Execution
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

# ST-001 — Stop a running agent turn

## Source Code

- `crates/desktop/src/shell/composer_panel.rs`, `crates/desktop/src/shell.rs`
- `crates/application/src/lib.rs`, `crates/application/src/queue.rs`
- `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`
- `docs/message-queue.md`

## Description

As a user, I want to stop an agent that is taking the wrong approach or running longer than needed, while retaining the conversation. Smaller Talk has queuing and steering but no user-facing turn cancellation. Cursor's Stop control ended the comparison turn and offered Resume.

## Outcome

An explicit Stop action ends the selected chat's active work and leaves a clear, recoverable state.

## Acceptance Criteria

- [ ] Stop is visible and keyboard reachable during an active turn, including while a provider interaction is pending.
- [ ] Cancellation targets the correct chat and turn; unrelated chats continue.
- [ ] Persisted partial output remains readable and the stopped state is distinct from success and failure.
- [ ] Pending follow-ups remain saved and paused; stopping cannot immediately dispatch another queued turn.
- [ ] Completion/cancellation races and rejected or unsupported interrupts leave an accurate state with recovery guidance.
- [ ] Owned processes are cleaned up or explicitly reported as still stopping; a later user message can continue without duplicate local messages.

## Scope

### In

- Native provider interruption, UI state, durable turn status, queue coordination, and recovery.

### Out

- Undoing file edits or external actions already performed; stopping unrelated processes.

## Implementation Notes

- Verify interruption semantics for both installed providers before selecting a transport. Killing a process and cancelling a native turn are different operations.
- Preserve the existing ownership boundary between provider adapters and application execution state.

## Dependencies / Blockers

- No prerequisite ticket identified. Provider interrupt and subprocess cleanup behavior require investigation during refinement.

## Questions

- Should subsequent work be called Continue, Resume, or a normal follow-up, depending on provider capabilities?
- What should a second Stop action do while shutdown is pending?

## Validation

- [ ] Exercise interruption during startup, streaming, tool work, and pending approval; include a simultaneous second chat and completion races.
- [ ] Restart after stopping and verify partial output, paused queues, and absence of duplicate delivery.
- [ ] Verify owned CLI/helper cleanup and settled memory after repeated stop cycles.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-002 Inspect agent activity and command results]]

## Notes

Backlog candidate. No priority or implementation commitment has been assigned.

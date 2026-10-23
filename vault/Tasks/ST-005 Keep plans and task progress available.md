---
id: ST-005
project: Smaller Talk
issue_type: Story
area: Planning
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

# ST-005 — Keep plans and task progress available

## Source Code

- `crates/desktop/src/shell.rs`, `crates/desktop/src/interactions.rs`
- `crates/desktop/src/transcript.rs`, `crates/domain/src/workspace.rs`
- `crates/application/src/lib.rs`, `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`
- `docs/chat-workspace.md`

## Description

As a user, I want to revisit and refine the current plan while discussing or implementing it, without repeatedly scrolling through chat. Smaller Talk already supports native Plan mode and explicit provider interactions. The proposed extension gives a plan and its task progress durable, accessible presentation.

## Outcome

A plan can be opened from chat, reviewed or refined, and followed during execution, with progress retained after switching chats or restarting.

## Acceptance Criteria

- [ ] A plan remains accessible through a compact chat affordance and a bounded optional view at wide and compact sizes.
- [ ] Clearly distinguish the current plan, revisions, and the revision authorized for execution.
- [ ] Edits are durably saved before execution; failed or pending saves cannot silently run an older plan.
- [ ] Leaving Plan mode retains the existing explicit choice; displaying or editing a plan does not authorize work.
- [ ] Display task status only when supported by provider events or a defined app-owned model; progress is not proof that acceptance criteria passed.
- [ ] Restore plan content, revision identity, and available progress after restart and chat switching without silently changing active work.

## Scope

### In

- Plan presentation, revision ownership, explicit execution handoff, and supported task progress.

### Out

- Reimplementing native provider planning policy, adding a general project manager, or requiring a Changes panel.

## Implementation Notes

- During the Cursor trial, edited task wording reverted on the Build transition. Treat persistence and revision handoff as explicit verification points.
- Decide whether edits are direct document changes, conversational refinements, or both before defining the provider contract.

## Dependencies / Blockers

- No prerequisite ticket identified. Structured plan/progress availability must be verified separately for each provider.

## Questions

- How should a new provider session or provider switch continue an existing plan?
- What does editing the plan during active implementation mean for the running turn?

## Validation

- [ ] Verify edit/save/build ordering, persistence failures, restart, provider switching, and plan revisions during active work.
- [ ] Exercise keyboard navigation and compact layouts without losing composer drafts or reading position.
- [ ] Benchmark repeated open/close of large plans and histories.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [Cursor Plan Mode](https://cursor.com/docs/agent/plan-mode)

## Notes

This extends an existing Plan mode; it does not treat planning itself as missing.

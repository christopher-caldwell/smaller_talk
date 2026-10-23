---
id: ST-011
project: Smaller Talk
issue_type: Spike
area: Commands
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

# ST-011 — Evaluate an embedded terminal

## Source Code

- `crates/desktop/src/shell.rs`, `crates/desktop/src/main.rs`, `crates/desktop/Cargo.toml`
- `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`
- `docs/chat-workspace.md`, `docs/memory-benchmark.md`

## Description

An embedded terminal was explicitly deferred. Inspecting the agent's command output may cover the immediate need; an interactive user shell is a separate capability with additional lifecycle and input responsibilities.

## Outcome

A decision record establishes whether command evidence, external-terminal handoff, or an embedded terminal best serves the user's workflows.

## Acceptance Criteria

- [ ] Identify concrete interactive shell needs beyond reading agent command results.
- [ ] Compare expandable command evidence, opening an external terminal in the effective folder, and an embedded PTY terminal.
- [ ] Define whether sessions belong to the user or the agent and how ownership differs from provider tool processes.
- [ ] Assess input/selection, resize, scrollback bounds, process cleanup, working-directory changes, restart expectations, and memory cost.
- [ ] Produce a build/defer recommendation and a minimal proposed scope; show measured evidence or explicitly label unresolved feasibility questions.

## Scope

### In

- Workflow research, lifecycle design options, and an optional isolated feasibility prototype if agreed during refinement.

### Out

- Shipping a terminal emulator, exposing interactive access to an agent-owned shell, or substituting a terminal for provider approvals.

## Implementation Notes

- [[ST-002 Inspect agent activity and command results]] provides noninteractive command evidence independently of this spike.
- Any prototype must account for both renderer/scrollback retention and its owned shell/helper processes.

## Dependencies / Blockers

- No prerequisite ticket identified. Agree representative commands and a time budget when selecting the investigation.

## Questions

- Is the user trying to inspect, rerun, or interact with commands?
- Should terminal sessions survive switching conversations, and who owns them after a chat closes?

## Validation

- [ ] Compare the agreed command workflows using evidence-only and external-terminal options.
- [ ] If prototyped, measure large-output bounds, resize/input behavior, repeated open/close memory, and child-process cleanup.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-002 Inspect agent activity and command results]]
- [[ST-001 Stop a running agent turn]]

## Notes

Deferred exploration candidate. A terminal is not a prerequisite for inspecting what an agent executed.

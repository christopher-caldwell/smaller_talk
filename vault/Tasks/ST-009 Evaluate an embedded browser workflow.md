---
id: ST-009
project: Smaller Talk
issue_type: Spike
area: Browser
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

# ST-009 — Evaluate an embedded browser workflow

## Source Code

- `crates/desktop/src/main.rs`, `crates/desktop/src/shell.rs`, `crates/desktop/Cargo.toml`
- `docs/chat-workspace.md`, `docs/memory-benchmark.md`

## Description

An embedded browser was explicitly deferred in the comparison. Before considering implementation, determine whether previewing or testing pages inside Smaller Talk solves a frequent user problem that existing external-browser workflows cannot address well enough.

## Outcome

A decision record recommends build, external integration, or continued deferral, with a small proposed scope and measured resource implications where feasible.

## Acceptance Criteria

- [ ] Identify concrete user workflows and success criteria rather than assuming a browser is needed because Cursor includes one.
- [ ] Compare opening external pages, a limited preview, and an embedded browser for interaction, maintenance, and native integration.
- [ ] Separate human browsing from agent browser control and specify whether either is proposed.
- [ ] Document process ownership, idle/active/closed memory, lifecycle, and compatibility with the settled app-memory goal; label unmeasured assumptions.
- [ ] Record the smallest useful option, unresolved decisions, and a build/defer recommendation. Create implementation tickets only after prioritization.

## Scope

### In

- Workflow research and, if selected during refinement, a small isolated feasibility prototype.

### Out

- Shipping a browser, importing profiles or credentials, or automatically granting agents browser access.

## Implementation Notes

- Prefer the existing memory benchmark conventions for any prototype comparison; include owned helper processes and preserve the same viewport/workload.
- An external-browser integration may satisfy the user need without retaining a browser inside the app.

## Dependencies / Blockers

- No prerequisite ticket identified. The user workflow and investigation time budget must be agreed when this spike is selected.

## Questions

- Is the need inspecting app output, reading research, or agent-driven end-to-end testing?
- Does content need to stay visible next to chat, or is a reliable external handoff sufficient?

## Validation

- [ ] Walk through the agreed workflow using the compared options and record friction and resource evidence.
- [ ] Review the recommendation against the native chat product direction and memory budget.

## Links

- [[Sources/2026-10-22 Cursor comparison]]

## Notes

Deferred exploration candidate. This ticket authorizes no browser implementation, installation, or account changes.

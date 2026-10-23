---
id: ST-008
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

# ST-008 — Display provider-reported context usage

## Source Code

- `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`
- `crates/application/src/lib.rs`, `crates/application/src/settings.rs`
- `crates/desktop/src/shell.rs`, `crates/desktop/src/shell/composer_panel.rs`
- `crates/domain/src/settings.rs`

## Description

As a user, I want to understand available context capacity and usage during long conversations, so I can interpret compaction or session changes. Cursor exposed an estimated usage percentage and category breakdown. Smaller Talk should present only data its providers actually expose.

## Outcome

A compact, optional usage display presents supported metrics with accurate units, timing, provenance, and unknown states.

## Acceptance Criteria

- [ ] Inventory and document the installed providers' supported usage, capacity, and compaction events before choosing displayed fields.
- [ ] Distinguish cumulative token usage from current context occupancy, and label any provider estimate as an estimate.
- [ ] Show a percentage only when numerator and capacity describe the same known context scope; otherwise show available values or Unavailable.
- [ ] Handle provider/model/session changes and compaction without displaying stale values as current.
- [ ] Associate persisted observations with the correct turn/session and distinguish them from next-turn predictions.
- [ ] The meter neither starts polling agent sessions nor invents category breakdowns or subscription/cost claims.

## Scope

### In

- Provider capability research, event mapping, bounded usage records, and compact display with optional explanation.

### Out

- A billing dashboard, fabricated token accounting, app memory telemetry, or exposing hidden prompts to estimate their size.

## Implementation Notes

- Start from advertised capabilities and protocol events. Unsupported metrics are a normal product state.
- Context inspection and numerical usage can share an entry point without requiring identical provider coverage.

## Dependencies / Blockers

- No prerequisite ticket identified. Availability and semantic meaning of provider data may limit this story's refined scope.

## Questions

- Which metrics are comparable between providers, and which must retain provider-specific labels?
- Should compaction appear as a historical event as well as a usage update?

## Validation

- [ ] Use fixtures for absent fields, partial updates, changing capacities, compaction, session reset, and stale concurrent events.
- [ ] Compare displayed values with one small authenticated run per supported provider.
- [ ] Confirm no background sessions remain solely to refresh the meter.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-007 Inspect effective conversation context]]

## Notes

Split from the context inspector so provider accounting uncertainty does not block visibility into known app-owned context.

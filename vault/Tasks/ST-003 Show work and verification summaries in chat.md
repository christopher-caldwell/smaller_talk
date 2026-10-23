---
id: ST-003
project: Smaller Talk
issue_type: Story
area: Review
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

# ST-003 — Show work and verification summaries in chat

## Source Code

- `crates/desktop/src/transcript.rs`, `crates/desktop/src/shell.rs`
- `crates/desktop/src/shell/repository.rs`
- `crates/application/src/repository.rs`, `crates/domain/src/repository.rs`
- `docs/settings-git-response-display.md`

## Description

As a user who rarely opens a Changes panel, I want to understand the result of an agent turn from the conversation itself. The initial comparison suggested connecting a turn summary to Changes; the user's correction makes an in-chat experience the primary requirement.

## Outcome

A compact completion summary shows what changed, what was verified, and what remains unresolved, with optional file previews and deeper review.

## Acceptance Criteria

- [ ] The conversation shows a useful work summary without opening a Changes panel.
- [ ] Observed edited files can be inspected individually from the summary; detailed repository review remains optional.
- [ ] Executed checks show command/result evidence when available, and distinguish failed, interrupted, unrun, and agent-reported checks.
- [ ] Clearly separate observed agent actions from all repository changes; a dirty working tree alone cannot establish turn ownership.
- [ ] Summaries remain associated with their historical turn after restart; unavailable historical evidence is labeled.
- [ ] Large file sets and previews are bounded, and expanding them does not disrupt reading or typing.

## Scope

### In

- In-chat summary and evidence links, optional bounded file/diff peeks, and explicit provenance.

### Out

- Requiring Changes as the main review workflow, automatic acceptance, stage/commit/push controls, or unsupported claims of exclusive file ownership.

## Implementation Notes

- Decide whether each summary item comes from provider action events, a repository observation, or model text; label those sources honestly.
- Reuse existing read-only repository capabilities where appropriate, without making the panel mandatory.

## Dependencies / Blockers

- [[ST-002 Inspect agent activity and command results]] is the likely source of durable action/check evidence. Confirm the minimal shared contract during refinement.
- A limited summary could precede full activity detail if its evidence limitations are explicit.

## Questions

- Which few facts are useful by default, and which should expand inline?
- How should non-coding work report artifacts and verification?

## Validation

- [ ] Complete a synthetic task and assess the result entirely from chat with Changes closed.
- [ ] Cover pre-existing user edits, external concurrent edits, no-edit turns, failed checks, large change sets, and restart.
- [ ] Verify keyboard access and memory behavior across repeated summary expansion.

## Links

- [[Sources/2026-10-22 Cursor comparison]]
- [[ST-002 Inspect agent activity and command results]]

## Notes

User preference: “I rarely view a changes panel.” This story has not been ranked above or below other candidates.

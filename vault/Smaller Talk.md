---
project: Smaller Talk
---

# Smaller Talk

Project-local product backlog. Open this `vault/` folder as an Obsidian vault.

## Source Code

The repository is the parent of this vault. Application sources are in `crates/`, product guides in `docs/`, and compact memory results in `benchmarks/results/`.

## Backlog

![[Smaller Talk Backlog.base]]

Open [[Smaller Talk Backlog.base]] directly for the full board. **By Status** follows Craft Code's Kanban columns and colors. **Triage** supports comparing candidates; **Ranked** shows tickets after a numeric rank is assigned; **All Tickets** includes completed work.

All eleven initial tickets are **Backlog**, with priority, rank, size, owner, and target unset. IDs identify tickets; their numbering does not assign importance. Eight stories describe possible product improvements. Three spikes retain the explicitly deferred browser, editor, and terminal ideas for later evaluation.

## Product Direction

- The user rarely opens a Changes panel. Work summaries and evidence should be useful directly in the conversation, with deeper file review optional.
- Preserve the native chat workflow and the memory goal of approaching 100 MiB when settled. Active agent work and transient peaks are measured separately.
- These are candidates for discussion, not approved implementation commitments. Prioritize after collecting the backlog.
- Plan mode, steering, queuing, skills, instructions, attachments, search, and read-only Git review already exist. Tickets describe extensions rather than treating those capabilities as missing.

## Working With Tickets

See [[Backlog Guide]] for triage, status transitions, adding tickets, and local Kanban setup. Start a new note from [[Templates/Ticket]] and assign the next unused ID; the next ID after this intake is **ST-012**.

The notes and Base configuration belong in Git. Personal window layout, trash, and plugin binaries stay local. No Jira account or sync is required.

## Sources

- [[Sources/2026-10-22 Cursor comparison]] — observations, user correction, and intake scope.

---
date: 2026-10-22
source_type: Hands-on comparison
project: Smaller Talk
---

# Cursor comparison — October 22, 2026

## Request and Correction

The user authorized using Cursor to brainstorm what Smaller Talk might be missing, without copying Cursor. After the comparison, the user requested a project-local Obsidian backlog modeled on the Craft Code Kanban Base, with Jira-style tickets, followed by a separate prioritization pass.

The user clarified: **“I rarely view a changes panel.”** The work-summary candidate therefore centers the conversation. Detailed file or diff inspection remains optional.

## Observed Workflow

A synthetic Python repository under ignored `.dev/cursor-comparison/` was used to attach a README through the context picker, generate a plan, begin implementation, steer additional edge cases, inspect changes and test output, view context usage, fork a conversation, and stop a separate turn. The sample implementation ended with nine passing tests. These were fixture tests, not Smaller Talk validation or a performance comparison.

Cursor exposed expandable file/search/command activity, an adjacent plan with task progress, a changed-files summary, a context breakdown, a fork control, and Stop/Resume controls. A plan-task edit appeared updated but reverted after Build began; this observation motivates verifying durable plan edits, rather than assuming the transition preserves them.

Smaller Talk's current implementation and guides were inspected. Existing capabilities include Plan mode, queued messages, steering, project folders, selected skills, instruction snapshots, attachments, and read-only repository diffs. The intake treats these as existing foundations.

## Captured Candidates

| Ticket | Candidate |
| --- | --- |
| [[Tasks/ST-001 Stop a running agent turn]] | Explicit turn cancellation with safe queue behavior. |
| [[Tasks/ST-002 Inspect agent activity and command results]] | Compact, expandable evidence of agent actions. |
| [[Tasks/ST-003 Show work and verification summaries in chat]] | Useful review without opening a Changes panel. |
| [[Tasks/ST-004 Reference workspace files and line selections]] | Searchable file and range references in prompts. |
| [[Tasks/ST-005 Keep plans and task progress available]] | Persistent, reviewable plans and progress. |
| [[Tasks/ST-006 Fork a conversation from a message]] | Preserve a discussion while exploring another approach. |
| [[Tasks/ST-007 Inspect effective conversation context]] | Folder, instructions, references, and session handoff visibility. |
| [[Tasks/ST-008 Display provider-reported context usage]] | Honest usage and context-capacity information. |
| [[Tasks/ST-009 Evaluate an embedded browser workflow]] | Deferred exploration. |
| [[Tasks/ST-010 Evaluate an embedded file editor]] | Deferred exploration. |
| [[Tasks/ST-011 Evaluate an embedded terminal]] | Deferred exploration. |

The browser, editor, and terminal were explicitly deferred in the comparison; they are recorded as spikes, not selected features. No earlier ranking is carried into this backlog. Additional ideas such as file checkpoints, native accessibility, or workspace isolation can be added separately rather than silently expanding this intake.

## Constraints and Evidence Limits

- The comparison used the installed Cursor Agents interface. It is a workflow observation, not a complete product audit.
- No new memory measurements were taken. Extra panels, logs, and context data need bounded retention and representative benchmarks before resource claims.
- App-owned memory and owned agent/helper processes remain separate measurements; active Codex memory growth is expected and must not be confused with idle app retention.
- No implementation work was authorized by creating these tickets.

## References

- [Cursor Plan Mode](https://cursor.com/docs/agent/plan-mode)
- [Cursor Agent overview](https://cursor.com/docs/agent/overview)
- Repository guides: `docs/chat-workspace.md`, `docs/message-queue.md`, `docs/custom-instructions.md`, `docs/settings-git-response-display.md`, `docs/skills-design-options.md`, and `docs/memory-benchmark.md`.

# Backlog Guide

## Open the Vault

In Obsidian, choose **Open folder as vault** and select this repository's `vault/` directory. Open [[Smaller Talk]] or [[Smaller Talk Backlog.base]].

The board uses **Kanban Bases View** (`kanban-bases-view`), matching the Craft Code vault. The local setup reuses the already-installed version 0.10.1 from Craft Code; it does not copy business notes or account settings. On another checkout, install Kanban Bases View from Obsidian's community plugin catalog and enable it in this vault. The built-in **Bases** plugin must also be enabled. The table views remain available without the community plugin.

Only this vault's lightweight configuration is tracked. Plugin binaries and Obsidian's window state are ignored. This vault is local to the project; no Obsidian Sync or Publish connection is configured.

On first open, Obsidian asks whether to trust the vault and enable its plugins. Enable the existing Kanban plugin when you are ready, or use the built-in table views in Restricted Mode. See [Obsidian's Bases syntax](https://obsidian.md/help/bases/syntax) for the underlying file format.

## Ticket Model

Membership comes from a Markdown note's location under `Tasks/`. The Base excludes source notes, this guide, and the template.

| Property | Meaning |
| --- | --- |
| `id` | Stable `ST-###` identifier. Never renumber to express priority or reuse a retired ID. |
| `issue_type` | `Story` for a possible user capability; `Spike` for research whose result may be a decision not to build. |
| `area` | Product area for comparison and filtering. |
| `candidate` | `Proposed` or `Deferred exploration`; neither is an implementation commitment. |
| `status` | Work state, using the workflow below. |
| `priority` | Leave blank until triage, then choose Low, Medium, High, or Urgent. |
| `rank` | Leave blank until ordered. A smaller positive number comes earlier in the Ranked view. |
| `size` | Leave blank until estimated; record the estimation convention before using it. |
| `owner`, `target` | Optional assignee and target date, assigned during planning. |
| `source` | Link to the originating discussion or evidence note. |

The initial intake deliberately overrides the usual Medium default: everything remains unprioritized until the user and team compare the candidates.

## Workflow

The columns match Craft Code:

| Status | Color | Use |
| --- | --- | --- |
| Backlog | Gray | Captured candidate awaiting triage or commitment. |
| To Do | Blue | Selected, scoped, and ready to start. |
| Doing | Orange | Implementation or investigation is underway. |
| Blocked | Red | Progress depends on a named decision or prerequisite. |
| Review | Purple | Outcome and validation evidence are ready for review. |
| Done | Green | Acceptance criteria and validation are satisfied. |

Dragging a Kanban card updates the note's `status`. Use `rank` to record agreed order across columns; board card position is only presentation order.

## Prioritization Session

1. Open **Triage** and discuss the user problem, frequency, impact, scope, dependencies, and memory cost for each candidate.
2. Refine or split tickets before estimating. A spike needs a concrete decision or report as its outcome.
3. Assign priority, size, and numeric rank only when agreed. Move selected work to **To Do**.
4. Resolve the ticket's Questions and name actual blockers. A useful integration is not automatically a hard prerequisite.
5. Before **Done**, record validation evidence and links to the implementation or decision.

For ST-003, review begins in chat. Opening Changes cannot be the only way to learn what happened or whether verification passed.

## Adding a Ticket

Create `Tasks/ST-### Short title.md`, insert [[Templates/Ticket]], and replace the ID, title, and empty sections. Check existing IDs before assigning a new one. Quick-add from the board creates a note in `Tasks/`; fill in the full ticket template and ID afterward.

Keep Source Code paths relative to the repository root so the notes survive moving or cloning the repo. Use vault-relative wiki links for related tickets. State dependencies explicitly in the body; a link alone does not imply ordering.

Do not mark a proposal implemented merely because a similar provider capability already exists. Record what Smaller Talk itself exposes and verifies.

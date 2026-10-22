# Skills in Smaller Talk

Implemented for issue #1. Skills belong to a message and retain the selected provider, exact source path, and resolution workspace. Saving a reference does not snapshot a skill's historical contents.

## Composer

Type `$` to search skills, or choose **+ → Skills…**. Both entry points use the same bounded list, with name, description, source, Refresh, and Inspect. Inspect reads instructions without running scripts. The composer displays editable `$name` text with subtle highlighting, rather than a permanent skill control or a separate chip row.

Enter accepts an active suggestion; it does not also send. Arrows navigate and Escape dismisses. Ordinary send/newline shortcuts work outside the picker. The editor's snapshots include skill identity, so undo and redo restore the selected source with the text. Deleting or changing a name removes its association; inserting a delimiter after it preserves the source.

Known names in prose resolve automatically when unambiguous. Duplicate Codex names require choosing a source. Escaped markers, currency, uppercase shell variables, shell expansion/assignment syntax, inline code, and fenced code remain literal. Unknown names remain ordinary text. Use an escape or code formatting when a literal lowercase variable would otherwise match a known skill.

Pasted `[$label](path)` links become visible mentions only when the exact path matches an available entry in the current catalog. Relative paths resolve against the effective workspace; local `file:` links are also accepted. Unknown links remain unchanged. A same-named skill at another path is never a substitute for a saved reference.

The native UI comparison used the regular highlighted editor and a temporary bolder prototype that deleted mentions atomically. The regular editor remains the default: it preserves familiar cursor editing and lets users correct names normally.

## Discovery and provider behavior

Application owns the skill service and catalog/inspection contracts. Infrastructure owns protocol details, filesystem metadata, provider configuration interpretation, and native command construction. Discovery uses the same effective workspace function as sending: the assigned project folder, falling back to the launch folder.

Catalogs are separate from model discovery and cached for 60 seconds by provider, configuration revision, and workspace. Opening the picker uses the cache or refreshes expired data; typing filters cached descriptors. Explicit refresh bypasses the cache. Provider/project generations reject stale UI results. Codex `skills/changed` invalidates cached results during an active connection; no permanent watcher process is required. Dispatch always revalidates selected references.

- **Codex:** discovery uses workspace-scoped `skills/list`. Initial input and steering use a shared builder that sends readable text, exact `{type: "skill", name, path}` references, and image inputs. Multiple distinct skills are supported; repeated references to the same source are deduplicated. Unsupported structured input fails without a text-only fallback. See [Codex app-server skills](https://learn.chatgpt.com/docs/app-server#skills).
- **Claude:** discovery reads configured personal, project/ancestor, and managed roots, legacy commands, enabled plugin install records, and declared plugin skill/command directories. JSONC settings and frontmatter visibility are respected. Skills that disable automatic invocation remain available for explicit invocation. Native precedence selects which source a command names; shadowed sources cannot be selected as if `/name` targeted their paths. At dispatch the exact source is checked again, and the CLI must advertise the command.
- **Claude transport:** one explicit skill per message. History, leading prose, file references, and images remain separate from the final `/command arguments` text block. Accepted inputs recovered for retry are replayed in chronological order as separate native messages. Permission requests retain the existing interaction workflow. This follows the noninteractive limitation described in [Claude's skills documentation](https://code.claude.com/docs/en/skills#stack-several-skills-in-one-message).

Native automatic skill discovery remains enabled. Only selections attached to current inputs, or confirmed inputs explicitly recovered for retry, become native invocation metadata. Historical transcript mentions remain context.

## Persistence and recovery

`Draft.skills` defaults to an empty list when older JSON is loaded. Drafts, the unsent new chat in session state, and durable queue payloads store references and mention ranges. Migration `0006_message_skills.sql` adds delivered-message metadata. Initial messages and accepted steering persist their metadata in the same transaction as the message and its existing associated records.

Queue acknowledgement and first-send recovery compare selected identities as well as visible text, so changing a source while submission completes cannot erase the revised draft. Retry loads the original references and confirmed steering metadata. Changes to an already delivered input's selection require a new message.

Unavailable, disabled, or mismatched references block delivery and pause queues with the provider's actionable reason. The composer offers Replace and Remove for unresolved sources. A queued entry's **Edit** action atomically moves its complete draft back to an empty composer, preserving references. An existing saved draft prevents that transfer; failed persistence rolls it back. The composer pauses editing while this short transfer completes; its draft writes and the transfer are serialized to prevent an older autosave from overwriting the recovered message.

Run `just db-migrate` before launching an existing installation. Ordinary startup still does not migrate the database.

## Verification and limits

Verification used macOS, Codex CLI 0.163.0, and Claude Code 2.1.303. Synthetic live skills contained response markers that were absent from the user prompts, distinguishing actual instruction use from payload assertions. Both adapters passed invocation, arguments, images, and resumed-session checks; Codex additionally passed a live two-skill invocation.

Deterministic tests cover protocol payloads, Claude precedence/visibility/plugins, exact-path resolution, cache scoping, Unicode edit ranges, legacy JSON, real SQLite draft and queue reopening, source-aware acknowledgements, atomic metadata rollback, accepted steering recovery, and failed queue-to-draft transfer. Native UI checks exercised autocomplete, Enter-to-select, highlighting, pasted exact-path links, Unicode paste, deletion/undo, native dead-key composition, restart persistence, and a synthetic invocation.

These checks establish the installed CLI behavior. Other versions must still accept the verified native transports; unsupported commands or selected references fail explicitly. Claude entries without a resolvable local source, including built-in commands without a source file, are not offered as exact-source skills. Native Japanese/Chinese IME sessions, remote filesystems, and every enterprise/plugin deployment were not exercised.

Installation, editing skill files, marketplaces, shared provider storage, and permanent chat-level selections are outside this feature.

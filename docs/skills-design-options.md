# Skills in Smaller Talk: design options

Research date: October 19, 2026. This is a proposal, not an implemented feature.

## Recommended: discover through the provider, select in the composer

Type `/` to search the active provider’s available skills. The existing `+` attachment control could also offer “Skills…” for discoverability; both entry points would open the same small picker. Show a name, short description, provider, and project/personal source. Selecting a skill adds a removable reference to the draft and leaves room for ordinary instructions and files. Start with one explicit skill per message, then consider multiple only after testing each provider’s actual expansion behavior.

The selection applies to that message. It should not become a permanent chat setting. Keep native automatic skill discovery separate: the agent may still use its configured skills without an explicit selection.

This fits the current chat interface without introducing a skill marketplace, editor, installation manager, or additional sidebar section.

## What the references establish

**Codex:** the app-server supports `skills/list` scoped by working directories. It can refresh discovery and emits `skills/changed` for invalidation. Explicit invocation supports a `$name` marker plus a structured `skill` input containing the name and SKILL.md path. Send that native reference rather than copying every skill body into each prompt. [Official OpenAI documentation](https://learn.chatgpt.com/docs/app-server#skills).

**Claude:** skills use `/name` invocation, with provider-specific configuration affecting discoverability and manual versus automatic activation. The catalog must respect `user-invocable`, `disable-model-invocation`, and effective visibility overrides. The native CLI should handle instruction expansion and skill execution. [Claude Code skills documentation](https://code.claude.com/docs/en/skills).

**T3:** its skill search filters user-invocable entries and ranks names/descriptions. Its Claude discovery code accounts for configuration roots, project roots, duplicate names, and overrides; its dispatch code translates references into native slash invocation. The dispatch source contains a one-skill assumption that differs from newer documented Claude stacking behavior, so it is an implementation reference rather than a protocol specification. [Search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/providerSkillSearch.ts), [Claude discovery](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/Drivers/ClaudeSkills.ts), [Claude dispatch](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/Drivers/ClaudeSkillDispatch.ts).

**Zed:** its own agent presents a lightweight catalog and loads bodies on demand. Its skill tool includes the skill directory when constructing context, so relative resources resolve correctly. Zed explicitly separates its own skills from external agents’ native skills. Smaller Talk runs external CLIs, so copy the catalog experience rather than Zed’s agent runtime. [Skill tool source](https://github.com/zed-industries/zed/blob/main/crates/agent/src/tools/skill_tool.rs), [Zed skills](https://zed.dev/docs/ai/skills), [External-agent boundaries](https://zed.dev/docs/ai/external-agents).

## Two smaller or broader alternatives

| Option | Benefit | Cost |
| --- | --- | --- |
| Native text invocation only | Users type Codex `$name` or Claude `/name`; minimal interface work. | Poor discovery, no source disambiguation, and no explicit validation. Text-only behavior through our adapters still needs live verification. |
| Shared app-owned skill folder | Potentially reusable instructions across providers. | Requires provider-specific root configuration, naming/override rules, and compatibility handling. A common folder does not guarantee equivalent provider behavior. Defer until cross-provider reuse is actually needed. |

## Implementation boundaries if approved

Add application-owned skill descriptors and draft selections. Keep protocol discovery, configuration interpretation, and native invocation in the provider adapters. Scope catalog caches by provider and effective project folder; refresh after folder/provider changes. Load descriptions first, with bounded search results, rather than retaining full bodies in GPUI views.

Record the invoked provider, skill identity, and source reference with the turn. Preserve selections through retries and draft restoration. If the user switches providers, show an unresolved reference until an explicit compatible choice is made; do not silently match by name. A shared name can resolve to different instructions.

Claude transport needs a focused live prototype: the current adapter combines prior conversation with the prompt, and native slash expansion depends on text-block placement. Confirm that skill arguments, project configuration, images, prior context, errors, and permission requests survive that transport before promising a generic cross-provider skill chip. Let the CLI execute any skill scripts under its existing policies; merely listing or previewing a skill must not execute them.

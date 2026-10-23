---
id: ST-012
project: Smaller Talk
issue_type: Story
area: Providers
candidate: Proposed
status: Backlog
priority:
rank:
owner:
target:
size:
created: 2026-10-22
source: "[[Sources/2026-10-22 CLI subscription usage research]]"
tags: [backlog]
---

# ST-012: Show subscription usage for configured CLIs

## Source Code

- `crates/infrastructure/src/codex.rs`, `crates/infrastructure/src/claude.rs`, `crates/infrastructure/src/catalog.rs`
- `crates/application/src/settings.rs`, `crates/domain/src/settings.rs`
- `crates/desktop/src/shell.rs`, `crates/desktop/src/shell/navigation.rs`, `crates/desktop/src/shell/settings.rs`
- `docs/settings-git-response-display.md`, `docs/memory-benchmark.md`

## Description

As a user, I want to see the remaining subscription allowance for my configured Claude and Codex CLIs without opening another app. A compact indicator in the lower left should open a usage popover, with full details and display preferences available in Settings.

The Claude desktop app provides the preferred interaction reference: a small usage button opens plan limits with session and weekly bars, reset timing, and a link to Settings. Smaller Talk should adapt that interaction to its sidebar and support both configured providers.

These are account subscription limits shared with other sessions and apps. Conversation context capacity belongs to [[ST-008 Display provider-reported context usage]].

## Outcome

Users can check available Claude and Codex subscription allowance, understand which window is constraining it, and see when the reading was captured and when the allowance resets.

## Acceptance Criteria

- [ ] Provide a compact usage entry in the lower left for enabled CLIs, with a usable entry when the sidebar is collapsed.
- [ ] Opening the entry shows each configured provider's available quota windows, explicitly labeled percentages, reset timing when supplied, and the observation time.
- [ ] If the compact display summarizes several overall quota windows, show the least remaining allowance and identify the relevant window in the details. Keep model allowances and monetary credits distinct.
- [ ] Settings → Providers & Models offers a sidebar visibility preference and usage reporting status for each provider; the popover links to these details.
- [ ] Collect Codex limits through `account/rateLimits/read` and Claude limits through the native `get_usage` control request on supported CLI versions. Reading usage sends no model prompt.
- [ ] Apply available live updates to the correct provider and account. Merge partial window updates without discarding other known windows.
- [ ] Cache and coalesce refreshes across chats. Provide explicit Refresh, bound automatic checks, and close any metadata probe after it completes or times out.
- [ ] Distinguish unsupported reporting, sign in requirements, temporary failures, and stale readings. Missing values never become zero, and missing reset times never become invented countdowns.
- [ ] Invalidate readings when the configured account or CLI configuration changes. Discard superseded responses and preserve dated readings only for the same account.
- [ ] Usage failures do not block chat, change provider readiness, or start a login flow. Opening and dismissing the popover preserves the draft, focus, and transcript position.

## Scope

### In

- Claude and Codex subscription limits, native metadata requests, supported stream updates, bounded caching, and refresh behavior.
- A compact sidebar entry, a Claude style popover, and provider settings for visibility and reporting status.
- Provider supplied session, weekly, and other applicable windows, with account scope and freshness made clear.

### Out

- Token or cost analytics, context occupancy, usage forecasts, or a separate analytics dashboard.
- A separate OAuth login, direct Keychain credential handling, browser cookie import, or terminal screen scraping in the first implementation.
- Buying credits, redeeming limit resets, changing billing settings, or automatically switching accounts or models.

## Implementation Notes

- Prefer T3's native collection approach. The installed Claude CLI answered `get_usage` during this discussion without receiving a model prompt; the method remains experimental and needs compatibility handling.
- Reuse an active CLI connection when supported. Otherwise run a short metadata probe, publish its result, and close it rather than retaining another idle agent process.
- Use an application owned usage snapshot and keep protocol mapping in Infrastructure. Include window identity, units, observation time, optional reset time, account identity, and reporting state.
- Normalize percentages carefully: T3 handles Claude `get_usage` utilization as 0–100 and streamed `rate_limit_event` utilization as 0–1. Codex exposes `usedPercent`.
- A five minute minimum between automatic checks is a starting point from T3, with explicit refresh and live events. Reconcile triggers during implementation and back off on throttling.
- Use Claude desktop for the interaction reference, CodexBar for failure and freshness behavior, and the Rust/GPUI claudebar for compact presentation. Preserve unknown data rather than copying defaults that turn missing fields into zero.
- Keep retained observations bounded and avoid loading transcript history to answer a subscription quota question.

## Dependencies / Blockers

- No prerequisite ticket identified. Authenticated CLI access and supported metadata requests determine provider coverage.
- ST-008 is related but independent; subscription reporting does not require a context meter.

## Questions

- Should sidebar usage be visible by default, and should a low allowance gain emphasis automatically?
- Which model allowances belong in the first popover, and should cloud credit balances remain a later addition?
- How should the UI identify an account when the CLI reports usage without a stable account identifier?

## Validation

- [ ] Use provider fixtures for missing fields, partial updates, percentage scales, absent resets, unsupported versions, failed probes, throttling, and account or configuration changes during refresh.
- [ ] Compare each supported provider's displayed limits with its native usage view on the same account. Record the CLI versions and capture times.
- [ ] Verify metadata probes send no model prompt, have bounded timeouts, and leave no extra CLI process running solely for usage display.
- [ ] Inspect expanded and collapsed sidebar states, keyboard activation, settings navigation, narrow windows, stale readings, and refresh during an active turn.
- [ ] Measure settled app memory and owned helper processes after repeated refreshes and popover use against the existing memory baseline.

## Links

- [[Sources/2026-10-22 CLI subscription usage research]]
- [[ST-008 Display provider-reported context usage]]
- [[Smaller Talk]]

## Notes

Captured from the discussion about configured CLI usage and Anthropic's Claude desktop app. This is a proposed backlog item. Priority, rank, estimate, owner, and target remain unset until triage.

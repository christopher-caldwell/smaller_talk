---
date: 2026-10-22
source_type: CLI probe and app comparison
project: Smaller Talk
---

# CLI subscription usage research: October 22, 2026

## Request

The user asked how to display usage for configured CLIs, suggested Settings or a small indicator in the lower left, and requested comparisons with T3 and other apps. The user then called attention to Anthropic's own Claude desktop app and asked to add the feature to the project tracker.

Captured as [[Tasks/ST-012 Show subscription usage for configured CLIs]]. This is separate from [[Tasks/ST-008 Display provider-reported context usage]], which concerns conversation capacity.

## Verified Observations

### Claude CLI

The installed CLI reported version `2.1.303`. A local metadata probe initialized a process with safe mode, no session persistence, no tools, and an empty strict MCP configuration. It sent a native `get_usage` control request with `skip_behaviors: true`, received a successful response with `rate_limits_available: true`, and closed the process.

Five hour and weekly windows contained utilization fields. The weekly window contained a reset time; the five hour reset was absent. No model prompt was sent. This verifies the installed CLI and account only; it does not establish compatibility with older versions or all authentication modes. No credentials or personal quota values are recorded here.

### Claude desktop app

The installed app reported version `3.1623.4`. Its Code mode exposed a small usage button beside the composer controls. Opening it showed a plan usage popover containing session and weekly progress bars, reset information, an available cloud credit balance, and a link to Settings → Usage. An unused session was labeled "Starts when a message is sent".

The live interface was inspected. Its internal network implementation was not traced. Anthropic's documentation states that Claude desktop, web, and Claude Code share account usage limits.

### T3 Code

Source inspection at commit `18e27425195686a5215fdc2cfa2bca088cdb8972` found a Usage button in the sidebar footer, a dedicated Limits view, and a `/usage-limits` command that opens details above the composer.

Codex discovery calls `account/rateLimits/read` with a separate three second usage deadline. Claude discovery initializes an SDK session without yielding a prompt, then calls `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET({ skipBehaviors: true })`, which uses the native `get_usage` request. Live provider events merge sparse usage updates into a shared snapshot.

The contract distinguishes unsupported reporting from a failed probe. Automatic refreshes wait at least five minutes between checks, including failed checks; users can request a refresh explicitly. These are source observations, not a local T3 runtime test.

### Other references

- CodexBar documents OAuth, CLI, and browser sources, account identity, credential expiry, stale readings, and throttling. Use it for failure handling; its broader credential machinery is outside the first proposed scope.
- `gautham-v/claudebar` uses Rust and GPUI, reads the existing Claude token, and calls the usage endpoint directly. Its compact presentation is relevant. Some missing utilization fields become zero in its parser, so that behavior should not be copied.
- `nerlichman/claudebar` combines OAuth and status line readings, chooses the fresher source, retains cached readings, and backs off after throttling. Its separate authentication and hooks are not needed for the preferred native CLI approach.

## Evidence Limits

- Smaller Talk has not implemented this feature. No new memory measurements were taken.
- Codex CLI version `0.163.0` was present, and its documented account limit protocol was reviewed; no authenticated Codex quota probe was run during this discussion.
- The native Claude request is experimental. Missing reset values and unsupported reporting must remain valid product states.
- Initial interactive concepts used illustrative values. They are sketches, not measured account data or an approved final layout.

## References

- [Codex app server account limits](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt)
- [T3 Claude probe](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/apps/server/src/provider/ClaudeProvider.ts#L343)
- [T3 Claude usage mapping](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/apps/server/src/provider/claudeUsageLimits.ts)
- [T3 Codex probe](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/apps/server/src/provider/CodexProvider.ts#L566)
- [T3 usage contract](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/packages/contracts/src/providerUsageLimits.ts)
- [T3 refresh behavior](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/packages/client-runtime/src/state/usage.ts#L46)
- [T3 usage guide](https://github.com/pingdotgg/t3code/blob/18e27425195686a5215fdc2cfa2bca088cdb8972/docs/user/usage.md)
- [Claude shared account limits](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work)
- [CodexBar Claude provider](https://github.com/steipete/CodexBar/blob/d4207d4f6626bfb0dd712da30fc100daf4b9f3ad/docs/claude.md)
- [Rust/GPUI claudebar](https://github.com/gautham-v/claudebar/tree/47f6f86078c3c744b7b61e038b2ee98b8c1106bf)
- [Rust claudebar parser](https://github.com/gautham-v/claudebar/blob/47f6f86078c3c744b7b61e038b2ee98b8c1106bf/src/usage.rs#L299)
- [ClaudeBar freshness and recovery](https://github.com/nerlichman/claudebar/blob/594757fad05c52fb357cbf7b11025eae9e0d0b42/README.md#how-it-works-local-first)

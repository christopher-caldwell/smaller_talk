# Recorded Codex response fixture

`codex-response-v1.md` is the exact response from the original prompt in the
[2026-10-22 Smaller Talk-only live run](../results/20261022T054648.450645Z-f5f468f6a230.json).
It contains 4,419 UTF-8 bytes and has SHA-256
`834c676700832d26df148da7449e6044ea138f6c029e2e86b4c3ddf82f38ad6f`.

Playback emits these bytes over the Codex stdio protocol through a local fixture
process. It does not run a model or measure the real Codex CLI's memory. It is an
application regression workload; the suite's separately labeled live job measures
the actual CLI and owned helper cost.

The fixed history workload stores 200 messages before launch, with the recorded
answer repeated four times in each of 100 assistant messages. Navigation and
closing use the app's normal handlers, and each independent trial starts with a
new database. This is a bounded stress workload, not a claim about typical history.

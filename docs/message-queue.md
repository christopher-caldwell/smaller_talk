# Message queue

Enter and Cmd+Enter send when a chat is idle and queue follow-ups while it is working. Shift+Enter always inserts a newline. The composer button changes from an arrow to a queue icon when submission will queue. There is no keyboard-hint footer.

Unsent entries appear in a compact strip immediately above the composer. Steer delivers an entry to the running native session; the trash button removes an unsent entry. The strip scrolls after three rows. At short heights, a pending provider interaction reduces it to one scrolling row and keeps a one-line composer available. Approval content itself scrolls so its actions remain reachable.

## Ownership and delivery

SQLite is the source of truth. Migration 0003 adds ordered queue entries, pause reasons, and associations between steered user messages and their active turn. Enqueue and removal of the matching saved draft happen in one transaction; a newer draft is retained. Attachment cleanup includes queue references. Deleting a chat cascades its queue, but removing queued attachments never modifies source files.

The application coordinator serializes claims and events independently of mounted tabs. It allows one turn per chat and different chats to run concurrently. Normal queue delivery waits for successful completion of the entire current turn, not a tool boundary. Provider, model, effort, Chat/Plan mode, and effective folder are resolved when dispatch starts. Queued content is absent from history, search, and provider handoffs until dispatched.

Queue previews in desktop memory are limited to 160 characters per entry and contain file metadata rather than decoded thumbnails. Each chat accepts at most 100 queued entries. Provider processes retain the existing bounded idle shutdown behavior.

## Recovery and steering

Queues restore paused after every restart and require Resume. Failed sends pause the queue. Entries that were dispatching or steering at shutdown become uncertain; review the conversation, then choose Retry or Remove before Resume. Retry only makes an uncertain entry eligible; it does not silently replay it. Native delivery cannot be made exactly-once across a process crash, so uncertain delivery is never automatically retried. Stable message IDs prevent duplicate local user messages on an explicit retry.

Questions, approvals, and requests to leave Plan mode block dispatch and steering. They keep their existing explicit response controls; queuing a prompt never answers or approves them.

Codex uses turn/steer with the expected active turn ID. Claude uses streaming JSON input with replayed user UUID acknowledgements and waits for every submitted input's result boundary. Steer is enabled only after the running adapter verifies the necessary native state. Unsupported or rejected steering remains queued with an actionable error. Unknown acknowledgements pause delivery rather than cancelling a turn or substituting an ordinary send.

Accepted steered messages are saved between assistant segments with their attachment associations. A successful provider acknowledgement alone does not remove the entry: the coordinator also waits for the persisted user-message event.

## Verification

- Ten queue integration tests cover FIFO, parallel chats without views, persistence, draft ownership and failed writes, attachment ownership, dispatch-time settings, missing files, approval blocking, uncertainty, and completion/steering races.
- Provider fixtures verify Codex expected-turn IDs and local-image inputs, Claude UUID acknowledgements and encoded images, and multiple result boundaries.
- The opt-in authenticated queue smoke test passed for both Codex and Claude using temporary databases/folders and synthetic prompts: steering was acknowledged, the remaining follow-up ran, and three user messages were saved without duplicates. No workspace files were created by either agent.
- Native checks exercised Enter/Cmd+Enter, Shift+Enter multiline drafts, deletion without draft loss, steering and typing focus, restart/Retry/Resume, two concurrent tabs, closing an active tab, and a compact approval with reachable Decline and disabled Steer.
- Debug RSS samples on the same 40-message long-history view at the default window size: previous build 128,608 KiB; queue build 126,976 KiB. A narrower two-turn/concurrent-tab sample used 70,992 KiB. These are individual observations affected by window geometry and macOS compression, not a stable benchmark or a claim that queuing reduces memory. Queue previews and the existing closed-view cache cleanup remain bounded.

Run `SMALLER_TALK_LIVE=1 SQLX_OFFLINE=true cargo test -p smaller-talk-desktop --test live_providers authenticated_queue_and_steering_both_providers --locked -- --ignored --nocapture` to repeat the authenticated queue smoke test.

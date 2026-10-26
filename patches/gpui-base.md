# GPUI Kit Base 0.8.0 app integration

The published `gpui-base` 0.8.0 package has two local changes:

- `src/input/base/state.rs`: expose `InputBaseState::transact`, using the existing
  undo manager and before/after selection snapshots. Selecting a skill inserts
  delimiter spaces and an identity-bearing inline token as one undoable edit.
- `src/input/base/element.rs`: ignore a deferred scroll offset clamped to stale
  content dimensions when fresh layout has changed them. Long pastes must reveal
  the caret after the composer reaches its row limit.

The app adapter regression tests exercise native history, token identity, and
long-paste caret visibility. Shortcut bindings and platform input handling remain
upstream. See `../../docs/gpui-kit-migration.md` for verification.

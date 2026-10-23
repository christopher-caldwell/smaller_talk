# Taffy 0.9.0 context cleanup backport

This is the published Taffy 0.9.0 crate used by GPUI 0.2.2, with the two cleanup
changes from [upstream PR #1181](https://github.com/DioxusLabs/taffy/pull/1181):
`TaffyTree::clear` clears node contexts, and `TaffyTree::remove` removes the
deleted node's context. Layout algorithms and public APIs are unchanged.

Without these changes, measurement closures retain captured text layouts after
GPUI clears its layout tree. Regression tests in
`crates/desktop/tests/layout_context.rs` check that both operations drop their
contexts before the tree itself is dropped. The tests fail against the original
published source.

The registry package checksum is
`a13e5d13f79d558b5d353a98072ca8ca0e99da429467804de959aa8c83c9a004`.
Upstream revision information is preserved in `.cargo_vcs_info.json`; the
original manifest is `Cargo.toml.orig`. Taffy is MIT licensed, as declared in
its manifest. The upstream license is included here.

Remove this override when GPUI supports a Taffy release containing the upstream
fix. Updating to the current Taffy API is a separate layout migration.

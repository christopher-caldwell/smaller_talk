# Taffy 0.13.0 context cleanup

This is the published Taffy 0.13.0 crate used by GPUI 0.3.9, with the existing
context-drop patch carried forward. `TaffyTree::clear` clears `node_context_data`;
`TaffyTree::remove` removes the deleted node's context. Layout algorithms and
public APIs remain upstream. The upstream MIT license is retained.

Without these changes, measurement closures can retain captured text layouts
when GPUI clears the tree. `crates/desktop/tests/layout_context.rs` checks context
drops against this exact version, also used by the production GPUI dependency.
Remove the override only after testing an upstream version with both fixes.

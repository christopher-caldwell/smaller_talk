# GPUI 0.3.9 transcript memory bounds

This directory contains the published `gpui-pre` 0.3.9 package. It replaces the
previous GPUI 0.2.2 fork. Upstream licenses and the original manifest are retained.

The previous memory controls are ported to the new source:

- `src/arena.rs`: trim high-water chunks and drop-list capacity after dropping
  allocations. Preserve the new upstream nested-scope and pointer-safety logic.
- `src/taffy.rs`: replace a layout tree after usage falls below one quarter of
  its peak above 4,096 nodes. Preserve the new backend's disabled rounding on
  the replacement. Bound the cleared layout bounds, outer origins, and computed
  layout containers using their latest usage.
- `src/window.rs`: trim oversized cleared element-state containers based on
  recent usage, with two times usage or at least 256 retained entries.
- `src/text_system.rs` and `src/text_system/line.rs`: four inline decoration runs
  per line instead of 32; complex lines still grow on the heap.

The Metal renderer moved out of GPUI. Its two-drawable limit is now retained in
`../gpui-apple/src/metal_renderer.rs`, patched into the production dependency graph.
Taffy's context cleanup remains in `../taffy`.

Frame-local IDs expire at the normal clear boundary. Ordinary and equally large
successive frames retain reusable capacity. Nested draws cannot free allocations
still used by outer draws. Regression tests compile the actual arena source;
layout-context and shaped-line tests use the linked runtime dependencies.

See `../../docs/gpui-kit-migration.md` for verification and memory observations.

# GPUI Apple 0.3.9 surface budget

The published `gpui-pre-apple` 0.3.9 package is retained with one source change:
`src/metal_renderer.rs` configures two CAMetalLayer drawables instead of three.
This carries forward the app's existing Metal surface memory budget after
GPUI's platform split. Upstream licenses and original manifest are retained.

Two drawables reduce queued-rendering headroom; native scrolling, streaming,
resizing, and fixed-history memory playback accompany this configuration.
See `../../docs/gpui-kit-migration.md` for verification.

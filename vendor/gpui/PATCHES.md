# GPUI 0.2.2 frame capacity reclamation

This is the published GPUI 0.2.2 crate, including its Apache 2.0 license. The
registry package checksum is
`979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`.
The published `.cargo_vcs_info.json` identifies revision
`69e2130295c2649963eb639fc70b4f2ee8ea1624` and marks that upstream checkout dirty;
the registry package, rather than that revision alone, is the source of truth.
`Cargo.toml.orig` preserves the upstream workspace manifest.

Only three source files are changed:

- `src/arena.rs`: after dropping elements and invalidating handles, release
  surplus chunks when a later frame uses less than a quarter of retained
  capacity (with a four-chunk floor). Trim an oversized element drop list too.
- `src/taffy.rs`: replace an oversized layout tree after node usage falls below
  a quarter of its previous peak, provided that peak exceeded 4,096 nodes.
  Layout IDs already expire at this end-of-frame boundary. Bound spare capacity
  in the layout bounds map and computed-layout set using their latest usage.
- `src/window.rs`: trim oversized cleared frame-state containers based on their
  most recent usage, retaining two times that usage or at least 256 entries.

Ordinary frames and equally large successive frames retain reusable storage.
No live frame state is evicted. Layout, text shaping, and list measurement are
unchanged. Opening large history still has a transient allocation peak; this
patch targets capacity retained after the large frame is gone. Alternating very
large and small frames can require more allocations, which should be considered
when assessing responsiveness.

`crates/desktop/tests/frame_arena.rs` exercises the actual arena source, including
upstream arena tests, destructor ordering, invalid old handles, subsequent
allocation, and reuse during repeated large frames. Native UI and memory runs
provide integration evidence for the private layout and frame code.

Remove this override when a compatible GPUI release provides bounded frame
capacity, after repeating the history and native UI checks.

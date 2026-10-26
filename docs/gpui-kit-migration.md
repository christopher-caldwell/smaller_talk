# GPUI Kit input migration

The desktop uses GPUI Kit's unstyled `gpui-base` 0.8.0 `Textarea`/`TextareaState` and `Input`/`InputState`. GPUI is pinned to the matching `gpui-pre` 0.3.9 snapshot; native platform construction comes from `gpui-pre-platform` 0.3.9. Rust 1.99.1 is pinned in `rust-toolchain.toml` and the workspace declares Rust 1.99. `just` prioritizes rustup over an older Homebrew compiler.

## Ownership and compatibility

`crates/desktop/src/composer.rs` is an application adapter. GPUI Kit owns character/word/line movement, selection, clipboard, edit history, Unicode/IME, wrapping, caret visibility, and scrolling. The adapter retains existing `Changed`, `Submitted`, `Canceled`, and picker-navigation events. Enter submits; Shift+Enter inserts a newline. The instructions editor continues to insert newlines on Enter. Programmatic changes to an unmounted field are queued until its next render, when a window is available to initialize and edit the native input state.

Drafts retain the existing text, UTF-8 selection endpoints, skill references, and attachment records. Token IDs encode the complete existing `SkillReference` (provider, workspace, path, name); restoration rebuilds token spans rather than resolving the name to a possibly different resource. The app continues to validate skills at dispatch and gate sending on unavailable references. Inline tokens use GPUI Kit's atomic selection/deletion and token-aware history. The skill picker, metadata warnings, attachment chips, thumbnails, model controls, and queue stay application-owned.

`gpui_base::Root` hosts the app's content and the toolkit's overlay infrastructure. The existing app palette and composer frame are retained. The adapter consumes the toolkit's propagated Enter action to prevent a second newline after submission.

## Vendored patch inventory

The sources come from the corresponding crates.io packages; their upstream licenses remain in each vendor directory. The workspace patches the actual packages used by the production desktop and the regression tests.

| Package | Local changes | Reason |
| --- | --- | --- |
| `gpui-pre` 0.3.9 | `src/arena.rs`: release high-water chunks and element storage inside `force_clear` after dropping elements; retain upstream scope-depth and pointer-safety logic. | Closed large histories must not pin large frame buffers. Nested draws must retain live allocations until the outer scope ends. |
| `gpui-pre` 0.3.9 | `src/taffy.rs` and `src/window.rs`: trim high-water layout trees, bounds/origin caches, and frame state after clearing. Preserve disabled rounding when replacing the new layout tree. | Retain the existing capacity bounds when large histories close. |
| `gpui-pre-apple` 0.3.9 | `src/metal_renderer.rs`: two drawables instead of three. | Carry forward the existing Metal surface budget after the renderer moved into a separate package. |
| `gpui-pre` 0.3.9 | `src/text_system.rs`, `src/text_system/line.rs`: four inline decoration runs instead of 32. | Bound per-line storage when shaping large transcripts. Complex lines can still grow on the heap. |
| `taffy` 0.13.0 | `TaffyTree::clear` and `remove` also clear/remove `node_context_data`. | Dropping layout nodes must release their measurement contexts. This remains necessary in 0.13.0. |
| `gpui-base` 0.8.0 | `InputBaseState::transact`: expose the existing edit transaction machinery with before/after selections. `layout_cursors`: ignore a deferred scroll target when fresh content dimensions differ. | Skill insertion may include delimiter spaces and a reference token; they must undo as one edit. A long paste must reveal its caret after growing to the row limit, rather than applying an offset clamped to the previous content size. Shortcut bindings and platform input handling remain upstream. |

Upstream versions and package paths are verified with `cargo tree -i gpui-pre` and `cargo tree -i taffy`. A GPUI upgrade must recheck all memory controls, not merely retain an unused patch for an older package version.

## Verification

The adapter has GPUI-driven tests for Option+Arrow movement and word selection in both input modes, clipboard selection replacement, undo/redo, multiline submission, skill identity across draft restore/deletion/undo, atomic skill insertion with delimiters, IME composition/commit, read-only editing, instructions-editor Enter behavior, and long-paste caret visibility at the row limit.

`frame_arena` compiles the exact vendored arena source, including its upstream tests. It verifies high-water release, reuse under equally large frames, stale-handle invalidation, and deferred release during nested draws. `layout_context` verifies context drops against the same patched Taffy version used by GPUI. `text_layout_memory` bounds the actual `ShapedLine` and `WrappedLine` types linked into the desktop.

The previous release binary and source snapshot were saved locally under `.dev/gpui-upgrade/` before migration. The deterministic benchmark uses separate databases, fixture responses, fixed 200-message histories, and three open/close cycles. The baseline was built with Rust 1.94.0; the candidate uses Rust 1.99.1. These busy-system observations include both framework and compiler changes and do not establish a performance regression verdict. Live provider execution is not required for these input and memory checks.

### Final checks (2026-10-23)

- `cargo test --workspace --locked`: 147 passed; six authenticated live-provider tests remain explicitly opt-in and were skipped.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check`, and the optimized desktop build pass.
- Native macOS checks used an isolated database and a local protocol fixture. Option+Arrow/Option+Shift+Arrow, replacement, undo/redo, Shift+Enter, skill completion/selection, attachment thumbnails, project-field Tab navigation, and the green project action were checked. A multiline Unicode draft with an exact skill reference and image survived a quit/relaunch. Enter dispatched exactly one request containing the expected text, structured skill path, and local image; the resulting chat retained its project assignment. An 18-line paste revealed line 18 within the seven-row viewport after the stale-scroll correction.
- The frame arena, layout-context drops, and shaped-line size regressions pass against the dependencies linked into the new desktop. The nested-frame arena check confirms that trimming cannot invalidate live outer-frame allocations.

Memory playback is observational on a busy machine and uses one trial per build. Local raw reports and verification evidence are under `.dev/gpui-upgrade/`; compact benchmark records with workload and build fingerprints are under `benchmarks/results/`.

The matched three-cycle playback records are [baseline](../benchmarks/results/20261023T224953.192111Z-27dd4d02d323.json) and [upgraded with every memory control ported](../benchmarks/results/20261023T231801.314752Z-e98c0bf47bae.json).

| App footprint, MiB | Baseline (Rust 1.94) | Upgraded (Rust 1.99.1) |
| --- | ---: | ---: |
| Before history, closed | 90.20 | 83.81 |
| History sampled peak | 269.70 | 319.16 |
| Closed after first cycle | 96.64 | 94.75 |
| Closed after third cycle | 96.67 | 101.91 |

The upgraded app releases the large history peak, and all existing capacity/context/surface controls remain active. Its transient history peak is higher, and the last closed state is about 5 MiB above baseline in this trial. These measurements do not establish that memory usage is identical or prove the absence of every leak. Earlier partial-port measurements under `.dev/gpui-upgrade/` are not the final build comparison. Native resizing, history scrolling, and streamed fixture responses also pass with the two-drawable Metal renderer restored.

An [extended eight-cycle trial](../benchmarks/results/20261023T231934.788379Z-55b134985043.json) with the same final binary completed successfully. Closed-history footprint medians were 94.60, 100.83, 97.33, 97.42, 97.25, 95.50, 102.03, and 97.24 MiB. There was no steadily increasing retention trend in those cycles; the final closed state was close to the original 96.67 MiB baseline. Its transient history peak was 315.85 MiB. This supports preservation of the existing memory bounds while retaining the higher-peak caveat above; it is not a general performance or leak-free verdict.

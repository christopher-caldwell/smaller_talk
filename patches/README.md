# Local dependency fixes

`vendor/` is a local copy of four upstream crates used by the workspace's
`[patch.crates-io]` overrides. Downloaded source is ignored by Git. Only the
application's changes, their notes, and pinned checksums live here.

Run `just deps` or `python3 scripts/prepare_dependencies.py` to restore the sources.
Python 3, `curl`, and `patch` are required. The setup script uses cached Cargo archives or
downloads exact versions from crates.io, verifies archive SHA-256 checksums,
applies these patches, and verifies the resulting source tree. It preserves the
upstream licenses and copies the patch notes to each restored directory.
`PATCHES.md` notes are excluded from source tree checksums.

| Directory | Package | Local changes |
| --- | --- | --- |
| `vendor/gpui` | `gpui-pre` 0.3.9 | [Frame, layout, and text memory bounds](gpui.md) |
| `vendor/gpui-apple` | `gpui-pre-apple` 0.3.9 | [Two Metal drawables](gpui-apple.md) |
| `vendor/gpui-base` | `gpui-base` 0.8.0 | [Input transactions and paste scrolling](gpui-base.md) |
| `vendor/taffy` | `taffy` 0.13.0 | [Drop removed layout contexts](taffy.md) |

Setup verifies existing directories and refuses to overwrite source edits. When
updating a patch, preserve your edited directory, regenerate the diff against the
pinned archive, and update `dependencies.json` with the new patched tree checksum.
When upgrading a crate, update its version and archive checksum too. Reconstruct
the dependencies in an empty directory with `--vendor-dir PATH`, compare them
with your working copies, and run the relevant regression tests before committing.

`Cargo.lock` stays tracked for reproducible resolution. `.sqlx/` stays tracked for
offline SQL query validation. Build outputs (`target/`), dependency copies
(`vendor/`, `node_modules/`, Python environments), caches, and local development
artifacts (`.dev/`) are ignored.

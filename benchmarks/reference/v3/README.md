# Original resource benchmark, v3

These three files are preserved byte-for-byte from the user's
`Downloads/smaller_talk_resource_benchmark_v3.zip`. `provenance.json` records the
archive and file SHA-256 values. They are a historical reference, including the
old app capability descriptions and preparation commands.

The current implementation is in `scripts/memory_bench/`. It carries forward
the original GUI bundle-ID verification, full app/backend process accounting,
manual `idle`, `work`, `post`, `snap`, `stop` workflow, CPU estimates, output
filenames, and Rust explanation prompt. See `docs/memory-benchmark.md` for current
commands and explicit changes in measurement behavior. The original files and
the archive in Downloads have not been edited.

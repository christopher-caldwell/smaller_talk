#!/usr/bin/env python3
"""Keep the original v3 --mode/--out CLI over the versioned collector."""

from memory_bench.cli import legacy_main

if __name__ == "__main__":
    raise SystemExit(legacy_main())

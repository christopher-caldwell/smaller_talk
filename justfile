# Use the committed query metadata without requiring a live build-time database.
export SQLX_OFFLINE := "true"
# Prefer rustup so Cargo and rustc both honor rust-toolchain.toml.
export PATH := env_var("HOME") + "/.cargo/bin:" + env_var("PATH")

# List available commands.
default:
    @just --list

# Restore pinned dependencies and apply the small tracked patches.
deps:
    python3 scripts/prepare_dependencies.py

# Format the entire workspace.
format: deps
    cargo fmt --all

# Check formatting without changing files.
format-check: deps
    cargo fmt --all -- --check

# Lint every workspace target, treating warnings as errors.
lint: deps
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Type-check every workspace target.
check: deps
    cargo check --workspace --all-targets --locked

# Run all workspace tests.
test: deps
    cargo test --workspace --locked

# Build an optimized release (target/release/smaller-talk-desktop).
build: deps
    cargo build --workspace --release --locked

# Launch the desktop app in debug mode.
run: deps
    cargo run --locked -p smaller-talk-desktop

# Restart the app on source changes (install with cargo install cargo-watch --locked).
dev: deps
    cargo watch -w crates -w Cargo.toml -w Cargo.lock -w .sqlx -x 'run --locked -p smaller-talk-desktop'

# Launch the optimized desktop app.
run-release: deps
    cargo run --release --locked -p smaller-talk-desktop

# Create the local SQLite database file.
db-create: deps
    cargo run --locked -p smaller-talk-desktop -- db-create

# Apply migrations to the existing SQLite database.
db-migrate: deps
    cargo run --locked -p smaller-talk-desktop -- db-migrate

# Create and migrate the local database before first launch.
db-setup: db-create db-migrate

# Remove Cargo build artifacts.
clean:
    cargo clean

# Collect desktop + execution-backend memory; use `just memory-bench --help`.
[positional-arguments]
memory-bench *args: deps
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/benchmark_memory.py "$@"

# Run automated Smaller Talk scenarios, including the real Codex CLI and helpers.
[positional-arguments]
bench *args: deps
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/benchmark_memory.py bench "$@"

# Verify collector integrity with small local probes, without provider calls.
memory-bench-test:
    PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest discover -s scripts -p 'test_*.py' -v

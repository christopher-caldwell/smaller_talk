# Use the committed query metadata without requiring a live build-time database.
export SQLX_OFFLINE := "true"

# List available commands.
default:
    @just --list

# Format the entire workspace.
format:
    cargo fmt --all

# Check formatting without changing files.
format-check:
    cargo fmt --all -- --check

# Lint every workspace target, treating warnings as errors.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Type-check every workspace target.
check:
    cargo check --workspace --all-targets --locked

# Run all workspace tests.
test:
    cargo test --workspace --locked

# Build an optimized release (target/release/smaller-talk-desktop).
build:
    cargo build --workspace --release --locked

# Launch the desktop app in debug mode.
run:
    cargo run --locked -p smaller-talk-desktop

# Restart the app on source changes (install with cargo install cargo-watch --locked).
dev:
    cargo watch -w crates -w Cargo.toml -w Cargo.lock -w .sqlx -x 'run --locked -p smaller-talk-desktop'

# Launch the optimized desktop app.
run-release:
    cargo run --release --locked -p smaller-talk-desktop

# Create the local SQLite database file.
db-create:
    cargo run --locked -p smaller-talk-desktop -- db-create

# Apply migrations to the existing SQLite database.
db-migrate:
    cargo run --locked -p smaller-talk-desktop -- db-migrate

# Create and migrate the local database before first launch.
db-setup: db-create db-migrate

# Remove Cargo build artifacts.
clean:
    cargo clean

# Collect desktop + execution-backend memory; use `just memory-bench --help`.
[positional-arguments]
memory-bench *args:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/benchmark_memory.py "$@"

# Run automated Smaller Talk scenarios, including the real Codex CLI and helpers.
[positional-arguments]
bench *args:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/benchmark_memory.py bench "$@"

# Verify collector integrity with small local probes, without provider calls.
memory-bench-test:
    PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest discover -s scripts -p 'test_*.py' -v

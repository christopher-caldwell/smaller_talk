# Smaller Talk

Smaller Talk is a macOS GPUI/Rust proof of concept for app-owned local chats. This phase provides the native chat list and durable empty conversations. Codex execution, streaming, Markdown rendering, and native thread resumption are planned for Phase 02.

## Local setup

The database lives at `~/Library/Application Support/SmallerTalk/chats.sqlite3`. Setup is explicit and separate from opening the app:

```sh
cargo run -p smaller-talk-desktop -- db-create
cargo run -p smaller-talk-desktop -- db-migrate
cargo run -p smaller-talk-desktop
```

Set `SMALLER_TALK_DATA_DIR` to choose another directory, for example a temporary directory for a local smoke check.

`db-create` creates the SQLite file. `db-migrate` applies the versioned schema to an existing file. Ordinary app startup never creates or migrates the database. SQLite has no PostgreSQL role/bootstrap machinery; the two local setup responsibilities remain separate.

The SQLx query macros validate external SQL files against the schema. Checked query metadata is committed under `.sqlx`, so normal builds do not require a live development database. To refresh that metadata after changing SQL or migrations, use SQLx CLI 0.8.6 with the local database selected:

```sh
DATABASE_URL=sqlite:///absolute/path/to/chats.sqlite3 cargo sqlx prepare --workspace
```

If SQLx CLI is not installed with SQLite support, install the matching CLI with `cargo install sqlx-cli --version 0.8.6 --no-default-features --features sqlite,rustls`.

## Workspace

- `smaller-talk-domain` owns app chat, message, and opaque execution binding values.
- `smaller-talk-application` owns chat use cases and the independent read/write ports.
- `smaller-talk-database` implements those ports with SQLite and feature-local checked SQL.
- `smaller-talk-desktop` composes the services and presents the GPUI window.

The POC uses the process working directory as its eventual provider context; there is no project picker or project catalog in this phase.

GPUI is pinned to 0.2.2 and uses its runtime Metal shader compilation option. The app still renders through Apple's native Metal path; the option avoids requiring the separate `metal` compiler during Cargo builds and moves shader compilation to application startup.

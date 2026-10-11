CREATE TABLE chats (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    adapter_key TEXT,
    native_session_id TEXT,
    CHECK ((adapter_key IS NULL) = (native_session_id IS NULL))
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY NOT NULL,
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX messages_chat_created_at ON messages(chat_id, created_at, id);

CREATE TABLE message_queue (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','dispatching','steering','uncertain','failed')),
    error TEXT
);
CREATE INDEX message_queue_chat_order ON message_queue(chat_id, ordinal);
CREATE TABLE queue_state (
    chat_id TEXT PRIMARY KEY REFERENCES chats(id) ON DELETE CASCADE,
    reason TEXT
);
CREATE TABLE turn_inputs (
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY(turn_id, message_id)
);

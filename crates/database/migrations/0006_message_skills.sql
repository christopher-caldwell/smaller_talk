CREATE TABLE message_skills (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    payload TEXT NOT NULL CHECK (json_valid(payload))
);

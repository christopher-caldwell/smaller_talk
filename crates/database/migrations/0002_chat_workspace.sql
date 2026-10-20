CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    folder TEXT
);
CREATE TABLE chat_settings (
    chat_id TEXT PRIMARY KEY NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    payload TEXT NOT NULL
);
CREATE INDEX chat_settings_project ON chat_settings(project_id);
CREATE TABLE drafts (
    chat_id TEXT PRIMARY KEY NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    payload TEXT NOT NULL
);
CREATE TABLE app_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payload TEXT NOT NULL
);
CREATE TABLE attachments (
    id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (id, message_id)
);
CREATE INDEX attachments_message ON attachments(message_id);
CREATE TABLE turns (
    id TEXT PRIMARY KEY NOT NULL,
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    user_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    settings TEXT NOT NULL,
    workspace TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'interrupted')),
    error TEXT
);
CREATE INDEX turns_chat ON turns(chat_id);

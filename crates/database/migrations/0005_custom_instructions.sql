CREATE TABLE project_instructions (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    text TEXT NOT NULL DEFAULT ''
);
ALTER TABLE chats ADD COLUMN instructions_fingerprint TEXT;
ALTER TABLE turns ADD COLUMN instructions TEXT;

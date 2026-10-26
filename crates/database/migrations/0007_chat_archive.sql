ALTER TABLE chats ADD COLUMN archived_at INTEGER;
CREATE INDEX chats_archived_at ON chats(archived_at);

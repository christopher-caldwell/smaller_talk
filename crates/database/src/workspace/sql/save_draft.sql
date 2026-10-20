INSERT INTO drafts (chat_id, payload)
SELECT id, ? FROM chats WHERE id = ?
ON CONFLICT(chat_id) DO UPDATE SET payload = excluded.payload;

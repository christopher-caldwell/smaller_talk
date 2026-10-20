INSERT INTO queue_state (chat_id, reason) SELECT id, ? FROM chats WHERE id = ?
ON CONFLICT(chat_id) DO UPDATE SET reason = excluded.reason;

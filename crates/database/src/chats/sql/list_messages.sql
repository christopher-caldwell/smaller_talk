SELECT id, chat_id, role, body, created_at
FROM messages
WHERE chat_id = ?
ORDER BY created_at ASC, id ASC;

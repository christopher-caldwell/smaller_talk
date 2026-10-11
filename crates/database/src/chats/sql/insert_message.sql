INSERT INTO messages (id, chat_id, role, body, created_at)
VALUES (?, ?, ?, ?, ?)
RETURNING id, chat_id, role, body, created_at;

INSERT INTO chats (id, title, created_at, adapter_key, native_session_id)
VALUES (?, ?, ?, ?, ?)
RETURNING id, title, created_at, adapter_key, native_session_id;

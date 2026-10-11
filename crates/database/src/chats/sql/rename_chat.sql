UPDATE chats
SET title = ?
WHERE id = ?
RETURNING id, title, created_at, adapter_key, native_session_id;

SELECT id, title, created_at, adapter_key, native_session_id, instructions_fingerprint
FROM chats
WHERE id = ?;

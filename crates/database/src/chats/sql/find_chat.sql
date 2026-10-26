SELECT id, title, created_at, adapter_key, native_session_id, instructions_fingerprint, archived_at
FROM chats
WHERE id = ?;

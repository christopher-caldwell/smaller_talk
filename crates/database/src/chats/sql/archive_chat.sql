UPDATE chats SET archived_at = ?
WHERE id = ? AND (? = 0 OR (
    NOT EXISTS (SELECT 1 FROM turns WHERE chat_id = chats.id AND status = 'running')
    AND NOT EXISTS (SELECT 1 FROM message_queue WHERE chat_id = chats.id)
))
RETURNING id, title, created_at, adapter_key, native_session_id, instructions_fingerprint, archived_at;

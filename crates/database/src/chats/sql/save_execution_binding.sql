UPDATE chats
SET adapter_key = ?, native_session_id = ?
WHERE id = ?
RETURNING id;

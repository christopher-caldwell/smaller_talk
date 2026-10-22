UPDATE chats
SET adapter_key = ?, native_session_id = ?, instructions_fingerprint = ?
WHERE id = ?
RETURNING id;

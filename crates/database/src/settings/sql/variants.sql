SELECT v.message_id, v.source_hash, v.provider, v.model, v.prompt_version, v.text
FROM response_variants v JOIN messages m ON m.id = v.message_id
WHERE m.chat_id = ? ORDER BY v.rowid

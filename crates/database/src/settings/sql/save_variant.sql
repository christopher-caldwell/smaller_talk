INSERT INTO response_variants (message_id, source_hash, provider, model, prompt_version, text)
VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(message_id, source_hash, provider, model, prompt_version) DO UPDATE SET text = excluded.text

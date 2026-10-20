INSERT INTO attachments (id, message_id, path, payload) VALUES (?, ?, ?, ?) ON CONFLICT(id, message_id) DO NOTHING;

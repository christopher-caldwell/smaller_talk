INSERT INTO global_settings (id, payload) VALUES (1, ?)
ON CONFLICT(id) DO UPDATE SET payload = excluded.payload

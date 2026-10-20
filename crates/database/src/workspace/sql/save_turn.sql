INSERT INTO turns (id, chat_id, user_message_id, settings, workspace, status, error) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET status = excluded.status, error = excluded.error;

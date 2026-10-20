INSERT INTO chat_settings (chat_id, project_id, payload) SELECT id, ?, ? FROM chats WHERE id = ? ON CONFLICT(chat_id) DO UPDATE SET project_id = excluded.project_id, payload = excluded.payload;

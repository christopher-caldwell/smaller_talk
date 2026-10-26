SELECT c.id AS chat_id, c.title, s.project_id, c.archived_at, p.name AS project, m.id AS message_id, m.body
FROM chats c
LEFT JOIN chat_settings s ON s.chat_id = c.id
LEFT JOIN projects p ON p.id = s.project_id
LEFT JOIN messages m ON m.chat_id = c.id
ORDER BY c.archived_at DESC, c.created_at DESC, c.id, m.created_at DESC, m.id;

SELECT s.project_id AS "project_id!: String", MAX(m.created_at) AS "updated_at?: i64"
FROM chat_settings s JOIN chats c ON c.id = s.chat_id JOIN messages m ON m.chat_id = c.id
WHERE s.project_id IS NOT NULL AND c.archived_at IS NULL GROUP BY s.project_id;

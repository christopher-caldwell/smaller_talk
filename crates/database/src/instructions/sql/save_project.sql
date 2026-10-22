INSERT INTO project_instructions (project_id, enabled, text)
SELECT id, ?, ? FROM projects WHERE id = ?
ON CONFLICT(project_id) DO UPDATE SET enabled = excluded.enabled, text = excluded.text;

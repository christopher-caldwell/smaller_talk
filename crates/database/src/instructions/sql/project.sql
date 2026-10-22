SELECT COALESCE(i.enabled, 0) AS "enabled!: i64", COALESCE(i.text, '') AS "text!: String"
FROM projects p LEFT JOIN project_instructions i ON i.project_id = p.id
WHERE p.id = ?;

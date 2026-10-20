SELECT ti.message_id
FROM turn_inputs ti
JOIN turns t ON t.id = ti.turn_id
JOIN messages m ON m.id = ti.message_id
WHERE t.chat_id = ?
  AND t.user_message_id = ?
  AND t.status IN ('interrupted', 'failed')
ORDER BY m.created_at, m.rowid;

SELECT instructions FROM turns t
WHERE t.chat_id = ? AND (t.user_message_id = ? OR EXISTS (
    SELECT 1 FROM turn_inputs i WHERE i.turn_id = t.id AND i.message_id = ?
)) ORDER BY t.rowid ASC LIMIT 1;

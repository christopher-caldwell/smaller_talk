DELETE FROM message_queue WHERE id = ? AND chat_id = ? AND state IN ('pending','failed','uncertain');

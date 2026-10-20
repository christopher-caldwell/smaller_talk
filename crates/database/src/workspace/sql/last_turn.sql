SELECT user_message_id, status, error FROM turns WHERE chat_id = ? ORDER BY rowid DESC LIMIT 1;

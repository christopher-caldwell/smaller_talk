SELECT id, chat_id, payload, state, error FROM message_queue WHERE chat_id = ? ORDER BY ordinal;

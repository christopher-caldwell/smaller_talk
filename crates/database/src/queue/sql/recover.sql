INSERT INTO queue_state (chat_id, reason) SELECT DISTINCT chat_id, 'Queue restored. Resume when ready.' FROM message_queue WHERE true
ON CONFLICT(chat_id) DO UPDATE SET reason = excluded.reason;

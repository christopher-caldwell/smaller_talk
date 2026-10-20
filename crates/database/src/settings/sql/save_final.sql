INSERT INTO final_answers (message_id, text) VALUES (?, ?)
ON CONFLICT(message_id) DO UPDATE SET text = excluded.text

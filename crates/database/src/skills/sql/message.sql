SELECT s.payload FROM message_skills s JOIN messages m ON m.id = s.message_id
WHERE m.chat_id = ? AND m.id = ?;

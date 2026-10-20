SELECT a.message_id, a.payload FROM attachments a JOIN messages m ON m.id = a.message_id WHERE m.chat_id = ? ORDER BY m.created_at, a.id;

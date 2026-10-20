SELECT f.message_id, f.text FROM final_answers f JOIN messages m ON m.id = f.message_id WHERE m.chat_id = ?

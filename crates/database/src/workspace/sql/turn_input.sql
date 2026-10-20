INSERT INTO turn_inputs (turn_id, message_id) VALUES (?, ?) ON CONFLICT DO NOTHING;

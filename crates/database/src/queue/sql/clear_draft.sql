DELETE FROM drafts WHERE chat_id = ?1
AND COALESCE(json_extract(payload, '$.skills'), '[]') = COALESCE(json_extract(?2, '$.skills'), '[]')
AND json_extract(payload, '$.text') = json_extract(?2, '$.text')
AND json_extract(payload, '$.attachments') = json_extract(?3, '$.attachments');

DELETE FROM drafts WHERE chat_id = ?
AND json_extract(payload, '$.text') = json_extract(?, '$.text')
AND json_extract(payload, '$.attachments') = json_extract(?, '$.attachments');

SELECT path AS "path!: String" FROM attachments
UNION SELECT json_extract(payload, '$.preview_path') FROM attachments WHERE json_extract(payload, '$.preview_path') IS NOT NULL
UNION SELECT json_extract(value, '$.path') FROM drafts, json_each(drafts.payload, '$.attachments')
UNION SELECT json_extract(value, '$.preview_path') FROM drafts, json_each(drafts.payload, '$.attachments') WHERE json_extract(value, '$.preview_path') IS NOT NULL
UNION SELECT json_extract(value, '$.path') FROM message_queue, json_each(message_queue.payload, '$.attachments')
UNION SELECT json_extract(value, '$.preview_path') FROM message_queue, json_each(message_queue.payload, '$.attachments') WHERE json_extract(value, '$.preview_path') IS NOT NULL;

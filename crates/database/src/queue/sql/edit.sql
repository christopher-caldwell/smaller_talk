DELETE FROM message_queue
WHERE chat_id = ?1 AND id = ?2 AND state IN ('pending', 'failed')
AND NOT EXISTS (
    SELECT 1 FROM drafts WHERE chat_id = ?1
    AND (length(json_extract(payload, '$.text')) > 0
        OR json_array_length(payload, '$.attachments') > 0
        OR COALESCE(json_array_length(payload, '$.skills'), 0) > 0)
)
RETURNING payload;

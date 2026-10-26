SELECT EXISTS(SELECT 1 FROM chats WHERE id = ? AND archived_at IS NOT NULL) AS "archived!: i64";

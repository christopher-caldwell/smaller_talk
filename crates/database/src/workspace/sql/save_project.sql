INSERT INTO projects (id, name, folder) VALUES (?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, folder = excluded.folder;

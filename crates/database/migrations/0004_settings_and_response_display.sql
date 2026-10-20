CREATE TABLE global_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payload TEXT NOT NULL
);
CREATE TABLE final_answers (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    text TEXT NOT NULL
);
CREATE TABLE response_variants (
    message_id TEXT NOT NULL REFERENCES final_answers(message_id) ON DELETE CASCADE,
    source_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version INTEGER NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (message_id, source_hash, provider, model, prompt_version)
);

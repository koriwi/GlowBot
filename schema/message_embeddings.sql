CREATE TABLE message_embeddings (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    embedding   BLOB NOT NULL,
    model       TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_embeddings_message ON message_embeddings(message_id);
CREATE INDEX idx_embeddings_model_message ON message_embeddings(model, message_id);

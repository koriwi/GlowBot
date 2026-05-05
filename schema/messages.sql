CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id     TEXT    NOT NULL,
    role        TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    reasoning   TEXT,
    name        TEXT,
    tool_calls  TEXT,
    tool_call_id TEXT,
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_messages_chat_created ON messages(chat_id, created_at);

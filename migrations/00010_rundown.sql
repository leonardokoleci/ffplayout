-- Continuous rundown queue ("rundown" processing mode):
-- ordered media items played back-to-back, independent of calendar dates.
CREATE TABLE rundown_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    source TEXT NOT NULL,
    seek REAL NOT NULL DEFAULT 0.0,
    out REAL NOT NULL DEFAULT 0.0,
    duration REAL NOT NULL DEFAULT 0.0,
    duration_audio REAL NOT NULL DEFAULT 0.0,
    title TEXT,
    ad INTEGER NOT NULL DEFAULT 0,
    custom_filter TEXT NOT NULL DEFAULT '',
    audio TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER
);

CREATE INDEX IF NOT EXISTS rundown_items_channel_position_idx
    ON rundown_items (channel_id, position);

CREATE TABLE rundown_state (
    channel_id INTEGER PRIMARY KEY REFERENCES channels (id) ON DELETE CASCADE,
    head_item_id INTEGER,
    head_start_epoch REAL NOT NULL DEFAULT 0.0,
    drained INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 0
);

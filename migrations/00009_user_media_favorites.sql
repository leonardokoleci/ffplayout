-- Per-user pinned media folders ("Quick Access"), scoped to a channel.
CREATE TABLE user_media_favorites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES auth_user (id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (user_id, channel_id, path)
);

CREATE INDEX IF NOT EXISTS user_media_favorites_user_idx ON user_media_favorites (user_id, channel_id);

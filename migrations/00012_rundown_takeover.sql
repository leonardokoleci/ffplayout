-- Rundown takeover flag: set when an operator command (jump, delete of the
-- live item) selected a new head. The playout iterator anchors the cursor at
-- the moment it actually serves that item and plays it from its beginning,
-- instead of seeking into it by the interrupt latency.
ALTER TABLE rundown_state ADD COLUMN takeover INTEGER NOT NULL DEFAULT 0;

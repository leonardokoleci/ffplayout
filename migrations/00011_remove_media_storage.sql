-- The static media storage path is gone. Media is browsed directly from any
-- disk or folder; users pin their preferred folders as favorites instead.
ALTER TABLE channels DROP COLUMN storage;

ALTER TABLE config_global DROP COLUMN storage;

-- Filler sources that were relative to the removed storage root are cleared;
-- absolute filler paths keep working.
UPDATE config_storage
SET filler = ''
WHERE filler NOT LIKE '%:%'
  AND substr(filler, 1, 1) NOT IN ('/', '\');

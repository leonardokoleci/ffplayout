-- Executable HTML graphics use a headless browser; the binary can be set explicitly.
ALTER TABLE config_processing
ADD COLUMN graphics_browser TEXT;

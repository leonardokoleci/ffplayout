-- Give existing channels a private, on-demand MoQ preview output.
--
-- The output is active so saved configurations discover it, but it is not
-- public and only publishes while a viewer holds a demand lease. Encoder
-- settings are shared from the channel's root HLS/stream output to avoid a
-- second encode. Channels that already have a MoQ output, a non-H.264/AAC
-- root encoder, or no active root encoder are left untouched.

CREATE TEMP TABLE preview_moq_owner AS
SELECT
    c.id AS config_id,
    owner.id AS owner_id,
    owner.name AS owner_name,
    owner.hls_playlist_name AS owner_playlist,
    owner.width,
    owner.height,
    owner.fps,
    COALESCE(owner.video_codec, 'libx264') AS video_codec,
    owner.video_options,
    owner.protocol_options,
    owner.muxer_options,
    owner.metadata_options,
    COALESCE(owner.audio_codec, 'aac') AS audio_codec,
    owner.audio_options,
    owner.audio_bitrate
FROM config c
JOIN config_output owner ON owner.config_id = c.id
WHERE
    owner.active = 1
    AND owner.name IN ('hls', 'stream')
    AND owner.encoding_source_id IS NULL
    AND COALESCE(owner.video_codec, 'libx264') = 'libx264'
    AND COALESCE(owner.audio_codec, 'aac') = 'aac'
    AND owner.id = (
        SELECT candidate.id
        FROM config_output candidate
        WHERE
            candidate.config_id = c.id
            AND candidate.active = 1
            AND candidate.name IN ('hls', 'stream')
            AND candidate.encoding_source_id IS NULL
            AND COALESCE(candidate.video_codec, 'libx264') = 'libx264'
            AND COALESCE(candidate.audio_codec, 'aac') = 'aac'
        ORDER BY
            CASE candidate.name WHEN 'hls' THEN 0 ELSE 1 END,
            candidate.priority,
            candidate.id
        LIMIT 1
    );

UPDATE config_output AS moq
SET
    active = 1,
    configured = 1,
    label = CASE WHEN moq.label = '' THEN 'Preview (MoQ)' ELSE moq.label END,
    share_encoding = 1,
    encoding_source_id = owner.owner_id,
    encoding_variant = CASE
        WHEN owner.owner_name = 'hls' THEN COALESCE(owner.owner_playlist, 'stream')
        ELSE ''
    END,
    public_moq = 0,
    priority = 1
FROM preview_moq_owner AS owner
WHERE
    moq.config_id = owner.config_id
    AND moq.name = 'moq'
    AND moq.active = 0
    AND moq.configured = 0
    AND NOT EXISTS (
        SELECT 1
        FROM config_output active_moq
        WHERE
            active_moq.config_id = moq.config_id
            AND active_moq.name = 'moq'
            AND active_moq.active = 1
    );

INSERT INTO config_output (
    config_id,
    active,
    name,
    label,
    priority,
    configured,
    inherit_resolution,
    share_encoding,
    encoding_source_id,
    encoding_variant,
    public_moq,
    hls_variants,
    stream_url,
    stream_type,
    stream_format,
    hls_playlist_name,
    hls_segment_duration,
    hls_list_size,
    desktop_fullscreen,
    width,
    height,
    fps,
    video_codec,
    video_options,
    protocol_options,
    muxer_options,
    metadata_options,
    audio_codec,
    audio_options,
    audio_bitrate
)
SELECT
    owner.config_id,
    1,
    'moq',
    'Preview (MoQ)',
    1,
    1,
    0,
    1,
    owner.owner_id,
    CASE
        WHEN owner.owner_name = 'hls' THEN COALESCE(owner.owner_playlist, 'stream')
        ELSE ''
    END,
    0,
    '',
    '',
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    0,
    owner.width,
    owner.height,
    owner.fps,
    owner.video_codec,
    owner.video_options,
    owner.protocol_options,
    owner.muxer_options,
    owner.metadata_options,
    owner.audio_codec,
    owner.audio_options,
    owner.audio_bitrate
FROM preview_moq_owner AS owner
WHERE
    NOT EXISTS (
        SELECT 1
        FROM config_output moq
        WHERE moq.config_id = owner.config_id AND moq.name = 'moq'
    )
    AND (
        SELECT COUNT(*)
        FROM config_output existing
        WHERE existing.config_id = owner.config_id
    ) < 9;

DROP TABLE preview_moq_owner;

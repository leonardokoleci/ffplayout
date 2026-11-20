use axum::{
    Json,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, header},
};
use protect_axum::authorities::AuthDetails;
use serde::{Deserialize, Serialize};

use crate::{
    api::{
        routes::{AuthUser, ensure_any_authority},
        state::AppState,
    },
    db::{
        handles,
        models::{Role, RundownItem, RundownState},
    },
    player::{
        controller::ChannelManager,
        utils::{Media, is_live_source},
    },
    utils::errors::ServiceError,
};

/// Duration assumed for live sources, which cannot be probed.
const LIVE_SOURCE_DEFAULT_DURATION: f64 = 60.0;

fn epoch_now() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

/// Payload for inserting a rundown item. `after_position` anchors the insert
/// (`None` appends at the end); `seek` is the media "in" point.
#[derive(Debug, Deserialize)]
pub struct NewRundownItem {
    #[serde(default)]
    pub after_position: Option<i64>,
    pub source: String,
    #[serde(default)]
    pub seek: Option<f64>,
    #[serde(default)]
    pub out: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub ad: Option<bool>,
    #[serde(default)]
    pub custom_filter: Option<String>,
    #[serde(default)]
    pub audio: Option<String>,
}

/// Payload for trimming/editing an existing rundown item.
#[derive(Debug, Deserialize)]
pub struct EditRundownItem {
    #[serde(default)]
    pub seek: Option<f64>,
    #[serde(default)]
    pub out: Option<f64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub ad: Option<bool>,
    #[serde(default)]
    pub custom_filter: Option<String>,
    #[serde(default)]
    pub audio: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MoveRundownItem {
    pub item_id: i64,
    #[serde(default)]
    pub after_position: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct JumpRundownItem {
    pub item_id: i64,
}

/// Payload for probing a source and deriving validated trim values without
/// inserting it into the queue.
#[derive(Debug, Deserialize)]
pub struct ProbeRundownSource {
    pub source: String,
    #[serde(default)]
    pub seek: Option<f64>,
    #[serde(default)]
    pub out: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
}

/// Validated trim values for a source, exactly as they would be stored.
#[derive(Debug, Serialize)]
pub struct RundownTrim {
    pub seek: f64,
    pub out: f64,
    pub duration: f64,
}

/// Payload for replacing the whole queue, e.g. from an imported playlist.
#[derive(Debug, Deserialize)]
pub struct RundownQueueImport {
    #[serde(default)]
    pub program: Vec<NewRundownItem>,
}

/// Pending metadata for the next M3U source line.
#[derive(Default)]
struct M3uEntry {
    duration: Option<f64>,
    title: Option<String>,
    seek: Option<f64>,
    out: Option<f64>,
}

/// Parse an M3U text: every non-comment line is a source. `#EXTINF` carries
/// the play duration and title; `#EXTFFP` is an ffplayout extension that
/// round-trips trimmed in/out points (other players ignore it).
fn parse_m3u(text: &str) -> Vec<NewRundownItem> {
    let mut items = Vec::new();
    let mut pending = M3uEntry::default();

    for line in text.lines() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let mut parts = info.splitn(2, ',');
            pending.duration = parts.next().and_then(|d| d.trim().parse::<f64>().ok());
            pending.title = parts
                .next()
                .map(|t| t.trim().replace(['\r', '\n'], " "))
                .filter(|t| !t.is_empty());
        } else if let Some(info) = line.strip_prefix("#EXTFFP:") {
            let mut parts = info.splitn(2, ',');
            pending.seek = parts.next().and_then(|v| v.trim().parse::<f64>().ok());
            pending.out = parts.next().and_then(|v| v.trim().parse::<f64>().ok());
        } else if line.starts_with('#') {
            continue;
        } else {
            // Without trim points the EXTINF play duration is the out point.
            let out = pending.out.or(pending.duration);

            items.push(NewRundownItem {
                after_position: None,
                source: line.to_string(),
                seek: pending.seek,
                out,
                duration: None,
                title: pending.title.take(),
                ad: None,
                custom_filter: None,
                audio: None,
            });

            pending = M3uEntry::default();
        }
    }

    items
}

/// The import body is either playlist JSON (engine format with a `program`
/// array, or a bare item array) or plain M3U/M3U8 text.
fn queue_from_body(
    content_type: Option<&str>,
    body: &[u8],
) -> Result<Vec<NewRundownItem>, ServiceError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| ServiceError::BadRequest("Import body is not valid UTF-8".into()))?
        .trim_start();

    let is_json = content_type.is_some_and(|ct| ct.starts_with("application/json"))
        || text.starts_with('{')
        || text.starts_with('[');

    if !is_json {
        return Ok(parse_m3u(text));
    }

    if text.starts_with('[') {
        return serde_json::from_str::<Vec<NewRundownItem>>(text)
            .map_err(|e| ServiceError::BadRequest(format!("Invalid playlist JSON: {e}")));
    }

    let import: RundownQueueImport = serde_json::from_str(text)
        .map_err(|e| ServiceError::BadRequest(format!("Invalid playlist JSON: {e}")))?;

    Ok(import.program)
}

/// A rundown item with the absolute timeline position the engine derived from
/// the persisted cursor.
#[derive(Debug, Serialize)]
pub struct RundownItemOut {
    pub id: i64,
    pub position: i64,
    pub source: String,
    #[serde(rename = "in")]
    pub seek: f64,
    pub out: f64,
    pub duration: f64,
    pub title: Option<String>,
    pub ad: bool,
    pub custom_filter: String,
    pub audio: String,
    /// Absolute UTC epoch second this item is scheduled to start.
    pub begin_epoch: f64,
    /// Absolute UTC epoch second this item is scheduled to end.
    pub end_epoch: f64,
    /// True when the engine has already consumed this item.
    pub played: bool,
}

/// The continuous queue with its playback cursor.
#[derive(Debug, Serialize)]
pub struct RundownQueue {
    pub revision: i64,
    pub head_item_id: Option<i64>,
    pub items: Vec<RundownItemOut>,
}

fn duration_of(item: &RundownItem) -> f64 {
    (item.out - item.seek).max(0.0)
}

/// Compute absolute epoch positions for every item, anchored at the head
/// cursor. Items before the head get past positions (played history). A queue
/// that never started is previewed from the current time.
fn with_timeline(state: &RundownState, items: Vec<RundownItem>) -> RundownQueue {
    let head_index = state
        .head_item_id
        .and_then(|head_id| items.iter().position(|item| item.id == head_id));

    let anchor = match head_index {
        Some(_) => state.head_start_epoch,
        None => epoch_now(),
    };

    let mut begins = vec![anchor; items.len()];

    if let Some(head_index) = head_index {
        let mut cursor = anchor;

        for index in head_index..items.len() {
            begins[index] = cursor;
            cursor += duration_of(&items[index]);
        }

        let mut cursor = anchor;

        for index in (0..head_index).rev() {
            cursor -= duration_of(&items[index]);
            begins[index] = cursor;
        }
    } else {
        // A queue that never started is previewed back-to-back from now.
        let mut cursor = anchor;

        for (index, item) in items.iter().enumerate() {
            begins[index] = cursor;
            cursor += duration_of(item);
        }
    }

    let items = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let played = head_index.is_some_and(|head| index < head);
            let item_duration = duration_of(&item);

            RundownItemOut {
                id: item.id,
                position: item.position,
                source: item.source,
                seek: item.seek,
                out: item.out,
                duration: item.duration,
                title: item.title,
                ad: item.ad,
                custom_filter: item.custom_filter,
                audio: item.audio,
                begin_epoch: begins[index],
                end_epoch: begins[index] + item_duration,
                played,
            }
        })
        .collect();

    RundownQueue {
        revision: state.revision,
        head_item_id: state.head_item_id,
        items,
    }
}

async fn channel_manager(state: &AppState, id: i32) -> Result<ChannelManager, ServiceError> {
    let manager = {
        let guard = state.controller.read().await;
        guard.get(id)
    }
    .ok_or_else(|| ServiceError::BadRequest(format!("Channel {id} not found!")))?;

    Ok(manager)
}

async fn build_queue(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    id: i32,
) -> Result<RundownQueue, ServiceError> {
    let queue_state = handles::select_rundown_state(pool, id).await;
    let items = handles::select_rundown_items(pool, id, None).await?;

    Ok(with_timeline(&queue_state, items))
}

/// Media sources are absolute paths; remote sources pass through unchanged.
fn absolute_source(source: &str) -> String {
    source.to_string()
}

/// Probe the source and derive the missing duration values. Live sources are
/// accepted without a probe.
async fn probe_rundown_source(
    source: &str,
    seek: Option<f64>,
    out: Option<f64>,
    duration: Option<f64>,
) -> Result<(f64, f64, f64), ServiceError> {
    let seek = seek.unwrap_or(0.0).max(0.0);

    if is_live_source(source) {
        let out = out.unwrap_or(seek + LIVE_SOURCE_DEFAULT_DURATION);

        return Ok((
            seek,
            out.max(seek),
            duration.unwrap_or(LIVE_SOURCE_DEFAULT_DURATION),
        ));
    }

    let mut media = Media::new(0, source, false).await;
    let probe_error = media.add_probe(true).await.err();

    let natural = media
        .probe
        .as_ref()
        .and_then(|probe| probe.format.duration)
        .filter(|probed| *probed > 0.0);

    let Some(natural) = natural else {
        return Err(ServiceError::BadRequest(format!(
            "Cannot probe source: {}{}",
            source,
            probe_error
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        )));
    };

    let out = out.unwrap_or(natural);
    let duration = duration.unwrap_or(natural);

    if out > duration {
        // Tolerate tiny rounding overshoots, e.g. from M3U EXTINF values
        // written with limited precision; anything larger stays an error.
        if out - duration > 0.1 {
            return Err(ServiceError::BadRequest(format!(
                "Out point {out} exceeds media duration {duration} for source {source}"
            )));
        }
    }

    Ok((seek, out.min(duration), duration))
}

/// Interrupt the currently playing clip so a cursor change takes effect
/// immediately. Serialized against engine replacement and live takeover the
/// same way the playlist control commands are.
async fn change_cursor_and_interrupt(
    manager: &ChannelManager,
    update: impl std::future::Future<Output = Result<(), ServiceError>>,
) -> Result<(), ServiceError> {
    let playback_control = manager.playback_control.lock().await;
    let navigation = playback_control
        .begin_navigation()
        .map_err(|blocked| ServiceError::Conflict(format!("Navigation blocked: {blocked:?}")))?;

    update.await?;
    // The committed navigation request interrupts the current clip. Do not
    // also call `skip_current`: it is consumed first and would leave the
    // navigation request pending, which then skips the takeover clip itself.
    navigation.commit();

    Ok(())
}

fn ensure_rundown_auth(
    user: &AuthUser,
    details: &AuthDetails<Role>,
    id: i32,
) -> Result<(), ServiceError> {
    ensure_any_authority(
        details,
        &[&Role::GlobalAdmin, &Role::ChannelAdmin, &Role::User],
    )?;
    user.ensure_channel_or_admin(id)?;

    Ok(())
}

/// #### ffplayout Rundown Operations
///
/// **Get the continuous queue**
///
/// ```BASH
/// curl -X GET http://127.0.0.1:8787/api/rundown/1
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// ```
pub async fn get_rundown(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
) -> Result<Json<RundownQueue>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;
    channel_manager(&state, id).await?;

    Ok(Json(build_queue(&state.pool, id).await?))
}

/// **Probe a source and derive validated trim values**
///
/// ```BASH
/// curl -X POST http://127.0.0.1:8787/api/rundown/1/probe
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"source": "/media/clip.mp4", "seek": 0}'
/// ```
pub async fn probe_rundown(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
    Json(data): Json<ProbeRundownSource>,
) -> Result<Json<RundownTrim>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;
    channel_manager(&state, id).await?;
    let source = absolute_source(&data.source);

    let (seek, out, duration) =
        probe_rundown_source(&source, data.seek, data.out, data.duration).await?;

    Ok(Json(RundownTrim {
        seek,
        out,
        duration,
    }))
}

/// **Insert a rundown item**
///
/// ```BASH
/// curl -X POST http://127.0.0.1:8787/api/rundown/1/item
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"source": "/media/clip.mp4"}'
/// ```
pub async fn add_rundown_item(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
    Json(data): Json<NewRundownItem>,
) -> Result<Json<RundownItemOut>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;

    // Validates that the channel exists.
    let _manager = channel_manager(&state, id).await?;
    let source = absolute_source(&data.source);

    let (seek, out, duration) =
        probe_rundown_source(&source, data.seek, data.out, data.duration).await?;

    let item = RundownItem {
        id: 0,
        channel_id: id,
        position: 0,
        source: source.clone(),
        seek,
        out,
        duration,
        duration_audio: 0.0,
        title: data.title,
        ad: data.ad.unwrap_or(false),
        custom_filter: data.custom_filter.unwrap_or_default(),
        audio: data.audio.unwrap_or_default(),
        created_at: 0,
        updated_at: None,
    };

    let inserted =
        handles::insert_rundown_item(&state.pool, id, data.after_position, &item).await?;
    handles::bump_rundown_revision(&state.pool, id).await?;

    let queue = build_queue(&state.pool, id).await?;

    queue
        .items
        .into_iter()
        .find(|item| item.id == inserted.id)
        .ok_or(ServiceError::InternalServerError)
        .map(Json)
}

/// **Edit a rundown item (trim in/out, title, etc.)**
///
/// ```BASH
/// curl -X PATCH http://127.0.0.1:8787/api/rundown/1/item/42
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"out": 120.5}'
/// ```
pub async fn edit_rundown_item(
    State(state): State<AppState>,
    AxumPath((id, item_id)): AxumPath<(i32, i64)>,
    user: AuthUser,
    details: AuthDetails<Role>,
    Json(data): Json<EditRundownItem>,
) -> Result<Json<RundownItemOut>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;

    let manager = channel_manager(&state, id).await?;

    let mut item = handles::select_rundown_item(&state.pool, id, item_id)
        .await?
        .ok_or_else(|| ServiceError::BadRequest(format!("Rundown item {item_id} not found")))?;

    if let Some(seek) = data.seek {
        item.seek = seek.max(0.0);
    }

    if let Some(out) = data.out {
        item.out = out;
    }

    if let Some(title) = data.title {
        item.title = Some(title);
    }

    if let Some(ad) = data.ad {
        item.ad = ad;
    }

    if let Some(custom_filter) = data.custom_filter {
        item.custom_filter = custom_filter;
    }

    if let Some(audio) = data.audio {
        item.audio = audio;
    }

    if item.out <= item.seek {
        return Err(ServiceError::BadRequest(
            "Out point must be greater than in point".to_string(),
        ));
    }

    if item.duration > 0.0 && item.out > item.duration {
        return Err(ServiceError::BadRequest(format!(
            "Out point {} exceeds media duration {}",
            item.out, item.duration
        )));
    }

    let queue_state = handles::select_rundown_state(&state.pool, id).await;
    let is_head = queue_state.head_item_id == Some(item.id);
    let mut interrupt = false;

    if is_head {
        // A trim that cuts into already-played material ends the clip now.
        let elapsed = epoch_now() - queue_state.head_start_epoch;

        if duration_of(&item) <= elapsed {
            interrupt = true;
        }
    }

    let update = async {
        handles::update_rundown_item(&state.pool, id, item_id, &item).await?;
        handles::bump_rundown_revision(&state.pool, id).await?;

        Ok(())
    };

    if interrupt {
        change_cursor_and_interrupt(&manager, update).await?;
    } else {
        update.await?;
    }

    let queue = build_queue(&state.pool, id).await?;

    queue
        .items
        .into_iter()
        .find(|item| item.id == item_id)
        .ok_or(ServiceError::InternalServerError)
        .map(Json)
}

/// **Delete a rundown item**
///
/// ```BASH
/// curl -X DELETE http://127.0.0.1:8787/api/rundown/1/item/42
/// -H 'Authorization: Bearer <TOKEN>'
/// ```
pub async fn delete_rundown_item(
    State(state): State<AppState>,
    AxumPath((id, item_id)): AxumPath<(i32, i64)>,
    user: AuthUser,
    details: AuthDetails<Role>,
) -> Result<Json<serde_json::Value>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;

    let manager = channel_manager(&state, id).await?;

    let queue_state = handles::select_rundown_state(&state.pool, id).await;
    let deleted = handles::select_rundown_item(&state.pool, id, item_id)
        .await?
        .ok_or_else(|| ServiceError::BadRequest(format!("Rundown item {item_id} not found")))?;

    let is_head = queue_state.head_item_id == Some(deleted.id);
    let successor = if is_head {
        handles::select_rundown_items(&state.pool, id, Some(deleted.position + 1))
            .await?
            .into_iter()
            .next()
    } else {
        None
    };

    let update = async {
        handles::delete_rundown_item(&state.pool, id, deleted.id).await?;

        if is_head {
            // The live item is gone: jump to the successor, or park the
            // cursor on the last remaining (already played) item.
            match successor {
                Some(next_item) => {
                    handles::set_rundown_head(
                        &state.pool,
                        id,
                        Some(next_item.id),
                        epoch_now(),
                        false,
                        true,
                    )
                    .await?;
                }
                None => {
                    let remaining = handles::select_rundown_items(&state.pool, id, None).await?;

                    match remaining.last() {
                        Some(last) => {
                            handles::set_rundown_head(
                                &state.pool,
                                id,
                                Some(last.id),
                                epoch_now(),
                                true,
                                false,
                            )
                            .await?;
                        }
                        None => {
                            handles::set_rundown_head(
                                &state.pool,
                                id,
                                None,
                                epoch_now(),
                                true,
                                false,
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        handles::bump_rundown_revision(&state.pool, id).await?;

        Ok(())
    };

    if is_head {
        change_cursor_and_interrupt(&manager, update).await?;
    } else {
        update.await?;
    }

    Ok(Json(
        serde_json::json!({ "revision": handles::select_rundown_state(&state.pool, id).await.revision }),
    ))
}

/// **Replace the whole rundown queue**
///
/// Used to import a playlist. The body is playlist JSON (engine format with
/// a `program` array, or a bare item array) or plain M3U/M3U8 text. Every
/// source is probed like a single insert; an empty `program` clears the
/// queue.
///
/// ```BASH
/// curl -X POST http://127.0.0.1:8787/api/rundown/1/queue
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"program": [{"source": "/media/clip.mp4"}]}'
/// ```
pub async fn replace_rundown_queue(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RundownQueue>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;

    let manager = channel_manager(&state, id).await?;

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let program = queue_from_body(content_type, &body)?;

    let mut items = Vec::with_capacity(program.len());

    for entry in &program {
        let source = absolute_source(&entry.source);
        let (seek, out, duration) =
            probe_rundown_source(&source, entry.seek, entry.out, entry.duration).await?;

        items.push(RundownItem {
            id: 0,
            channel_id: id,
            position: 0,
            source,
            seek,
            out,
            duration,
            duration_audio: 0.0,
            title: entry.title.clone(),
            ad: entry.ad.unwrap_or(false),
            custom_filter: entry.custom_filter.clone().unwrap_or_default(),
            audio: entry.audio.clone().unwrap_or_default(),
            created_at: 0,
            updated_at: None,
        });
    }

    let queue_state = handles::select_rundown_state(&state.pool, id).await;

    let update = async {
        handles::replace_rundown_queue(&state.pool, id, &items).await?;

        Ok(())
    };

    if queue_state.head_item_id.is_some() {
        // The channel is playing from the old queue: stop the current clip
        // so the engine picks up the replacement right away.
        change_cursor_and_interrupt(&manager, update).await?;
    } else {
        update.await?;
    }

    Ok(Json(build_queue(&state.pool, id).await?))
}

/// **Move a rundown item**
///
/// ```BASH
/// curl -X POST http://127.0.0.1:8787/api/rundown/1/move
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"item_id": 42, "after_position": 3}'
/// ```
pub async fn move_rundown_item(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
    Json(data): Json<MoveRundownItem>,
) -> Result<Json<RundownQueue>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;
    channel_manager(&state, id).await?;

    handles::move_rundown_item(&state.pool, id, data.item_id, data.after_position).await?;
    handles::bump_rundown_revision(&state.pool, id).await?;

    Ok(Json(build_queue(&state.pool, id).await?))
}

/// **Jump to a rundown item**
///
/// Stops the currently playing clip and starts the given item immediately.
///
/// ```BASH
/// curl -X POST http://127.0.0.1:8787/api/rundown/1/jump
/// -H 'Content-Type: application/json' -H 'Authorization: Bearer <TOKEN>'
/// --data '{"item_id": 42}'
/// ```
pub async fn jump_rundown_item(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    user: AuthUser,
    details: AuthDetails<Role>,
    Json(data): Json<JumpRundownItem>,
) -> Result<Json<RundownQueue>, ServiceError> {
    ensure_rundown_auth(&user, &details, id)?;

    let manager = channel_manager(&state, id).await?;

    handles::select_rundown_item(&state.pool, id, data.item_id)
        .await?
        .ok_or_else(|| {
            ServiceError::BadRequest(format!("Rundown item {} not found", data.item_id))
        })?;

    let update = async {
        // The takeover flag makes the playout iterator anchor the cursor when
        // it actually serves the item, so the clip starts at its beginning
        // regardless of how long the interrupt takes.
        handles::set_rundown_head(
            &state.pool,
            id,
            Some(data.item_id),
            epoch_now(),
            false,
            true,
        )
        .await?;
        handles::bump_rundown_revision(&state.pool, id).await?;

        Ok(())
    };

    change_cursor_and_interrupt(&manager, update).await?;

    Ok(Json(build_queue(&state.pool, id).await?))
}

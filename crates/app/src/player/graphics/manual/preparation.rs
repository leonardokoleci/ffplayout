use std::{path::Path, sync::atomic::Ordering};

use anyhow::{Context, Result};
use ff_engine::ClipGraphics;
use ff_graphics::{
    playback::Transition,
    playlist::{Action, ClipSchedule, Event, Limits},
    preparation::PreparationService,
};
use serde_json::Value;

use crate::{player::controller::ChannelManager, utils::errors::ServiceError};

use super::super::{microseconds, prepare_scene_budgeted};

pub(super) async fn prepare(
    manager: &ChannelManager,
    data: Value,
) -> Result<ClipGraphics, ServiceError> {
    let permit = manager
        .manual_jobs
        .clone()
        .try_acquire_owned()
        .map_err(|_| ServiceError::Conflict("Graphics preparation busy; retry shortly".into()))?;
    let config = match manager.running_config() {
        Some(config) => (*config).clone(),
        None => manager.config.read().await.clone(),
    };
    let canvas = config
        .program
        .as_ref()
        .map_or((config.output.width, config.output.height), |program| {
            (program.width, program.height)
        });
    let alive = manager.is_alive.clone();
    let state = manager.manual_state.clone();
    let epoch = state.epoch.load(Ordering::Acquire);
    let preparation = manager.graphics.preparation();
    // Graphics sources resolve relative to the shared browser anchor; pickers
    // normally deliver absolute paths from any disk.
    let root = manager.storage.root.read().await.clone();
    let browser = config
        .processing
        .graphics_browser
        .as_ref()
        .map(std::path::PathBuf::from);

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepare_manual(
            data,
            &root,
            canvas,
            config.general.channel_id,
            || !alive.load(Ordering::SeqCst) || state.epoch.load(Ordering::Acquire) != epoch,
            preparation,
            browser,
        )
    })
    .await?
    .map_err(|error| ServiceError::BadRequest(format!("Cannot prepare graphic: {error:#}")))
}

/// Reuse playlist validation/preparation for one manual program layer. Unlike a
/// clip schedule, an untimed manual graphic holds until explicitly stopped.
fn prepare_manual(
    data: Value,
    root: &Path,
    canvas: (u32, u32),
    channel: i32,
    cancelled: impl Fn() -> bool,
    preparation: PreparationService,
    browser: Option<std::path::PathBuf>,
) -> Result<ClipGraphics> {
    let (renderer, mut schedule) = prepare_scene_budgeted(
        &Value::Array(vec![data.clone()]),
        root,
        super::super::SceneRequest {
            canvas,
            duration_us: i64::MAX / 4,
            channel,
            browser,
        },
        cancelled,
        Some(preparation),
    )?;

    if data.get("duration").is_none_or(Value::is_null)
        && data.get("hold").is_none_or(Value::is_null)
    {
        let scene = schedule
            .frame_at(Some(0))
            .context("manual scene unavailable")?
            .scene;
        let layer = scene
            .layers
            .first()
            .context("graphic source could not be prepared")?;
        let transition = Transition {
            fade_in_us: microseconds(data.get("fade_in").and_then(Value::as_f64).unwrap_or(0.0))?
                as u64,
            fade_out_us: microseconds(data.get("fade_out").and_then(Value::as_f64).unwrap_or(0.0))?
                as u64,
            ..Transition::default()
        };
        schedule = ClipSchedule::compile(
            i64::MAX / 4,
            &[Event {
                at_us: 0,
                action: Action::Take {
                    layer: layer.spec,
                    transition,
                },
            }],
            std::slice::from_ref(&layer.resource),
            Limits::default(),
        )?;
    }

    Ok(ClipGraphics::new(renderer, schedule))
}

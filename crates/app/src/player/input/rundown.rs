//! Continuous rundown source iterator ("rundown" processing mode).
//!
//! Plays an ordered queue of media items back-to-back, independent of calendar
//! dates and a fixed day start. The playback cursor is persisted in the
//! database (`rundown_state`): the item that is currently live and the absolute
//! UTC epoch second it started. Every item's scheduled window is derived from
//! that cursor, so the queue seamlessly crosses midnight and accepts edits at
//! any time: inserted, trimmed, deleted and jumped-to items are picked up at
//! the next clip boundary.

use std::path::Path;

use chrono::{Timelike, Utc};
use log::*;

use crate::{
    db::{handles, models::RundownItem},
    player::{
        controller::ChannelManager,
        input::playlist::{apply_init_seek, fill_source_fallback},
        utils::{
            Media, get_date, is_close, is_live_source, is_remote,
            json_serializer::normalize_media_clip, time_in_seconds,
        },
    },
    utils::config::PlayoutConfig,
};

/// When the wall clock runs this far ahead of (or behind) the derived schedule
/// after advancing the cursor, the schedule is rebased onto the current time.
const RESYNC_THRESHOLD: f64 = 2.0;

/// Duration of the placeholder clip served while the queue is empty.
const FILLER_DURATION: f64 = 60.0;

/// How often the iterator recomputes the queue when a concurrent operator
/// command changed the cursor while it was serving.
const SERVE_ATTEMPTS: usize = 4;

fn epoch_now() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.0
}

/// Drop queue entries whose scheduled window is already over. Returns the
/// (possibly rebased) cursor and how many entries were skipped. Entries whose
/// window is still running or upcoming are kept.
fn skip_overdue_windows(durations: &[f64], mut cursor: f64, now: f64) -> (f64, usize) {
    let mut skipped = 0;

    for &duration in durations {
        let window_end = cursor + duration;

        if window_end > now {
            break;
        }

        skipped += 1;

        if now - window_end > RESYNC_THRESHOLD {
            // The wall clock ran far ahead of the derived schedule; rebase it
            // so appended items get fresh positions.
            cursor = now;
        } else {
            cursor = window_end;
        }
    }

    (cursor, skipped)
}

/// Iterator for the rundown queue. One call to `next()` hands out exactly one
/// clip; the persisted cursor moves forward whenever a window is over.
#[derive(Debug)]
pub struct RundownSource {
    channel_id: i32,
    config: PlayoutConfig,
    manager: ChannelManager,
    current_node: Media,
    last_node_ad: bool,
    /// Id of the head item whose clip was handed out most recently.
    served_head_id: Option<i64>,
}

/// A queue entry together with the playout clip prepared from it.
struct QueueEntry {
    item: RundownItem,
    media: Media,
}

impl QueueEntry {
    fn duration(&self) -> f64 {
        self.media.out - self.media.seek
    }
}

impl RundownSource {
    pub async fn new(manager: ChannelManager) -> Self {
        let config = manager.config.read().await.clone();
        let channel_id = config.general.channel_id;

        info!(channel = channel_id; "Playout in rundown mode");

        *manager.current_date.lock().await = get_date(false, 0.0, false, &config.channel.timezone);

        Self {
            channel_id,
            config,
            manager,
            current_node: Media::default(),
            last_node_ad: false,
            served_head_id: None,
        }
    }

    /// Apply filler resolution for sources the engine cannot play. Items carry
    /// their probed duration from the insert, so existing sources are handed
    /// to the engine directly and clip boundaries stay immediate; only missing
    /// sources fall back to filler.
    async fn prepare_node(&mut self, node: &mut Media) {
        let duration = node.out - node.seek;

        if node.duration > 0.0 && duration < 1.0 {
            warn!(
                channel = self.channel_id;
                "Skip clip that is less then one second long (<span class=\"log-number\">{duration:.3}</span>)."
            );
            node.skip = true;
        }

        if self.source_is_available(&node.source) {
            return;
        }

        fill_source_fallback(&self.manager, &self.config, self.channel_id, node, true).await;
    }

    /// Local files are checked on disk. Live and remote sources were accepted
    /// without a probe on insert and are handed to the engine as-is; probing
    /// them here could block for seconds on an unreachable URL.
    fn source_is_available(&self, source: &str) -> bool {
        source.is_empty()
            || is_live_source(source)
            || is_remote(source)
            || Path::new(source).is_file()
    }

    /// Serve a placeholder while the queue is empty. The engine keeps polling,
    /// so an item appended to the empty queue starts at the next boundary.
    async fn serve_filler(&mut self) {
        let mut node = Media::new(0, "", false).await;
        node.begin = Some(time_in_seconds(&self.config.channel.timezone));
        node.duration = FILLER_DURATION;
        node.out = FILLER_DURATION;
        node.is_placeholder = true;
        node.last_ad = self.last_node_ad;

        self.prepare_node(&mut node).await;

        self.current_node = node;
    }

    /// Build a playout clip from a database row.
    async fn queue_entry(&self, item: RundownItem, index: usize) -> QueueEntry {
        let mut media = Media::new(index, &item.source, false).await;
        media.seek = item.seek;
        media.out = item.out;
        media.duration = item.duration;
        media.duration_audio = item.duration_audio;
        media.title = item.title.clone();
        media.ad = item.ad;
        media.custom_filter = item.custom_filter.clone();
        media.audio = item.audio.clone();
        media.index = Some(index);

        normalize_media_clip(&mut media);

        QueueEntry { item, media }
    }

    pub async fn next(&mut self) -> Option<Media> {
        self.last_node_ad = self.current_node.last_ad;

        // A concurrent operator command can change the cursor between reading
        // the state and writing it back. Recompute with the fresh state so a
        // takeover is never overwritten by the iterator.
        for attempt in 0..SERVE_ATTEMPTS {
            let exhausted = attempt + 1 == SERVE_ATTEMPTS;

            match self.serve_queue(exhausted).await {
                ServeOutcome::Node(node) => return Some(*node),
                ServeOutcome::Stale if exhausted => break,
                ServeOutcome::Stale => {}
            }
        }

        error!(
            channel = self.channel_id;
            "Rundown cursor kept changing while serving, serve filler"
        );
        self.serve_filler().await;
        self.served_head_id = None;

        Some(self.current_node.clone())
    }

    /// Read the queue and hand out the next clip. Returns `Stale` when the
    /// persisted cursor changed between reading and writing it back.
    async fn serve_queue(&mut self, ignore_stale: bool) -> ServeOutcome {
        let pool = self.manager.db_pool.clone();
        let state = handles::select_rundown_state(&pool, self.channel_id).await;

        let head_position = match state.head_item_id {
            Some(head_id) => handles::select_rundown_item_position(&pool, self.channel_id, head_id)
                .await
                .ok()
                .flatten(),
            None => None,
        };

        // While drained, only items appended after the parked cursor matter.
        let from_position = match (head_position, state.drained) {
            (Some(position), true) => Some(position + 1),
            (position, _) => position,
        };

        let rows = handles::select_rundown_items(&pool, self.channel_id, from_position).await;

        let Ok(rows) = rows else {
            error!(channel = self.channel_id; "Cannot read rundown queue, serve filler");
            self.serve_filler().await;
            self.served_head_id = None;

            return ServeOutcome::Node(Box::new(self.current_node.clone()));
        };

        let mut queue = Vec::with_capacity(rows.len());

        for (index, item) in rows.into_iter().enumerate() {
            queue.push(self.queue_entry(item, index).await);
        }

        let now = epoch_now();
        let mut cursor = state.head_start_epoch;

        // The queue is drained: resume with freshly appended items at the
        // current time, or keep serving filler until then.
        if state.drained {
            if queue.is_empty() {
                self.serve_filler().await;
                self.served_head_id = None;

                return ServeOutcome::Node(Box::new(self.current_node.clone()));
            }

            cursor = now;
        } else if state.head_item_id.is_none() {
            // A fresh queue (no cursor yet) starts at the current time.
            if queue.is_empty() {
                self.serve_filler().await;
                self.served_head_id = None;

                return ServeOutcome::Node(Box::new(self.current_node.clone()));
            }

            cursor = now;
        } else if state.takeover {
            // An operator command selected this head for immediate playback.
            // Anchor the cursor at the moment the clip is actually served: the
            // file starts at its beginning no matter how long the interrupt of
            // the previous clip took, the countdown counts from the real
            // start, and the item can no longer be skipped as overdue.
            cursor = now;
        } else if self.served_head_id == state.head_item_id {
            // The head was already handed out, so its window has now been
            // played or skipped: move the cursor past it.
            let played = queue.remove(0);
            cursor += played.duration();
        }

        // Fast-forward over windows that are already over (downtime, restart,
        // appends after a drained queue).
        let durations = queue.iter().map(QueueEntry::duration).collect::<Vec<f64>>();
        let (advanced, skipped) = skip_overdue_windows(&durations, cursor, now);

        for overdue in queue.drain(..skipped) {
            warn!(
                channel = self.channel_id;
                "Skip overdue rundown item <span class=\"log-addr\">{}</span>",
                overdue.media.source
            );
        }

        cursor = advanced;

        if queue.is_empty() {
            // The queue is drained: park the cursor behind the last window so
            // a restart does not replay it, and keep serving filler until new
            // items are appended.
            if state.head_item_id.is_some()
                && !self
                    .persist_head(&pool, state.head_item_id, cursor, true, &state)
                    .await
                && !ignore_stale
            {
                return ServeOutcome::Stale;
            }

            self.serve_filler().await;
            self.served_head_id = None;

            return ServeOutcome::Node(Box::new(self.current_node.clone()));
        }

        let elapsed = (now - cursor).max(0.0);
        let head = queue.remove(0);
        let head_id = head.item.id;

        if !self
            .persist_head(&pool, Some(head_id), cursor, false, &state)
            .await
            && !ignore_stale
        {
            return ServeOutcome::Stale;
        }

        let mut node = head.media;
        node.begin = Some(day_seconds(cursor, &self.config.channel.timezone));

        if elapsed > 0.0 {
            debug!(
                channel = self.channel_id;
                "Rundown starts <span class=\"log-number\">{elapsed:.3}</span> seconds into the scheduled item"
            );
            apply_init_seek(&mut node, elapsed);
        }

        node.last_ad = self.last_node_ad;

        self.prepare_node(&mut node).await;

        self.served_head_id = Some(head_id);
        self.current_node = node;

        let pending = queue.into_iter().map(|entry| entry.media).collect();

        *self.manager.current_list.lock().await = pending;

        ServeOutcome::Node(Box::new(self.current_node.clone()))
    }

    /// Persist the playback cursor. Skipped when nothing changed, so clip
    /// boundaries do not cause needless revision bumps. Returns `false` when a
    /// concurrent operator command changed the cursor first.
    async fn persist_head(
        &self,
        pool: &sqlx::Pool<sqlx::Sqlite>,
        head_item_id: Option<i64>,
        head_start_epoch: f64,
        drained: bool,
        state: &crate::db::models::RundownState,
    ) -> bool {
        let unchanged = head_item_id == state.head_item_id
            && drained == state.drained
            && !state.takeover
            && is_close(head_start_epoch, state.head_start_epoch, 0.001);

        if unchanged {
            return true;
        }

        match handles::advance_rundown_head(pool, state, head_item_id, head_start_epoch, drained)
            .await
        {
            Ok(written) => {
                if !written {
                    debug!(
                        channel = self.channel_id;
                        "Rundown cursor changed by an operator command while serving"
                    );
                }

                written
            }
            Err(e) => {
                error!(channel = self.channel_id; "Cannot persist rundown cursor: {e}");

                true
            }
        }
    }
}

/// Result of one queue-serving attempt.
enum ServeOutcome {
    Node(Box<Media>),
    Stale,
}

/// Convert an absolute epoch second into seconds-of-day in the channel
/// timezone, so log and status output stay comparable to playlist mode.
fn day_seconds(epoch: f64, tz: &Option<chrono_tz::Tz>) -> f64 {
    let millis = (epoch * 1000.0).round() as i64;
    let Some(point) = chrono::DateTime::from_timestamp_millis(millis) else {
        return 0.0;
    };
    let local = point.with_timezone(tz.as_ref().unwrap_or(&chrono_tz::Tz::UTC));
    let seconds = local.time().num_seconds_from_midnight() as f64;

    seconds + millis.rem_euclid(1000) as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use chrono_tz::Tz;

    use super::{RESYNC_THRESHOLD, day_seconds, skip_overdue_windows};

    #[test]
    fn keeps_a_still_running_window() {
        let (cursor, skipped) = skip_overdue_windows(&[100.0, 50.0], 1000.0, 1050.0);

        assert_eq!(skipped, 0, "the first window is still running at now");
        assert_eq!(cursor, 1000.0);
    }

    #[test]
    fn skips_consecutive_overdue_windows_without_rebase() {
        // Each skipped window is less than the rebase threshold behind the
        // clock, so the cursor advances window by window.
        let (cursor, skipped) = skip_overdue_windows(&[100.0, 1.0], 0.0, 101.5);

        assert_eq!(skipped, 2);
        assert_eq!(cursor, 101.0);
    }

    #[test]
    fn rebases_when_the_clock_ran_far_ahead() {
        let (cursor, skipped) = skip_overdue_windows(&[100.0, 100.0], 0.0, 5000.0);

        assert_eq!(skipped, 1, "after a rebase no further window is overdue");
        assert_eq!(cursor, 5000.0);
    }

    #[test]
    fn rebase_threshold_is_respected() {
        let (cursor, _) = skip_overdue_windows(&[100.0], 0.0, 100.0 + RESYNC_THRESHOLD);

        assert_eq!(cursor, 100.0);

        let (cursor, _) = skip_overdue_windows(&[100.0], 0.0, 100.0 + RESYNC_THRESHOLD + 0.1);

        assert_eq!(cursor, 100.0 + RESYNC_THRESHOLD + 0.1);
    }

    #[test]
    fn midnight_crossing_is_plain_epoch_arithmetic() {
        // 23:59:59 UTC on 2025-11-24, then 00:00:01 UTC the next day.
        let late = day_seconds(1_764_028_799.0, &Some(Tz::UTC));
        let early = day_seconds(1_764_028_801.0, &Some(Tz::UTC));

        assert!((late - 86_399.0).abs() < 0.002);
        assert!((early - 1.0).abs() < 0.002);
    }
}

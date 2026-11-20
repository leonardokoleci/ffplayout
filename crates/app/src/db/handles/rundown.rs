use sqlx::{Pool, Sqlite};

use crate::{
    db::models::{RundownItem, RundownState},
    utils::errors::ProcessError,
};

pub async fn select_rundown_state(pool: &Pool<Sqlite>, channel_id: i32) -> RundownState {
    sqlx::query_as::<_, RundownState>(
        "SELECT channel_id, head_item_id, head_start_epoch, drained, takeover, revision
        FROM rundown_state
        WHERE channel_id = $1",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(RundownState {
        channel_id,
        head_item_id: None,
        head_start_epoch: 0.0,
        drained: false,
        takeover: false,
        revision: 0,
    })
}

pub async fn select_rundown_items(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    from_position: Option<i64>,
) -> Result<Vec<RundownItem>, ProcessError> {
    const QUERY: &str = "SELECT id, channel_id, position, source, seek, out, duration,
        duration_audio, title, ad, custom_filter, audio, created_at, updated_at
        FROM rundown_items
        WHERE channel_id = $1 AND position >= $2
        ORDER BY position, id";

    let items = sqlx::query_as::<_, RundownItem>(QUERY)
        .bind(channel_id)
        .bind(from_position.unwrap_or(0))
        .fetch_all(pool)
        .await?;

    Ok(items)
}

pub async fn select_rundown_item(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    item_id: i64,
) -> Result<Option<RundownItem>, ProcessError> {
    const QUERY: &str = "SELECT id, channel_id, position, source, seek, out, duration,
        duration_audio, title, ad, custom_filter, audio, created_at, updated_at
        FROM rundown_items WHERE channel_id = $1 AND id = $2";

    let item = sqlx::query_as::<_, RundownItem>(QUERY)
        .bind(channel_id)
        .bind(item_id)
        .fetch_optional(pool)
        .await?;

    Ok(item)
}

pub async fn select_rundown_item_position(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    item_id: i64,
) -> Result<Option<i64>, ProcessError> {
    let position: Option<i64> =
        sqlx::query_scalar("SELECT position FROM rundown_items WHERE channel_id = $1 AND id = $2")
            .bind(channel_id)
            .bind(item_id)
            .fetch_optional(pool)
            .await?;

    Ok(position)
}

/// Bump the revision counter so engine and UI detect a queue change.
pub async fn bump_rundown_revision(
    pool: &Pool<Sqlite>,
    channel_id: i32,
) -> Result<i64, ProcessError> {
    const QUERY: &str = "INSERT INTO rundown_state (channel_id, revision) VALUES ($1, 1)
        ON CONFLICT(channel_id) DO UPDATE SET revision = revision + 1
        RETURNING revision";

    let revision: i64 = sqlx::query_scalar(QUERY)
        .bind(channel_id)
        .fetch_one(pool)
        .await?;

    Ok(revision)
}

/// Store the playback cursor and bump the revision. Operator commands (jump,
/// delete of the live item) set `takeover`, which tells the playout iterator to
/// anchor the cursor when it actually serves the item.
pub async fn set_rundown_head(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    head_item_id: Option<i64>,
    head_start_epoch: f64,
    drained: bool,
    takeover: bool,
) -> Result<i64, ProcessError> {
    const QUERY: &str =
        "INSERT INTO rundown_state (channel_id, head_item_id, head_start_epoch, drained, takeover, revision)
        VALUES ($1, $2, $3, $4, $5, 1)
        ON CONFLICT(channel_id) DO UPDATE SET
            head_item_id = excluded.head_item_id,
            head_start_epoch = excluded.head_start_epoch,
            drained = excluded.drained,
            takeover = excluded.takeover,
            revision = revision + 1
        RETURNING revision";

    let revision: i64 = sqlx::query_scalar(QUERY)
        .bind(channel_id)
        .bind(head_item_id)
        .bind(head_start_epoch)
        .bind(drained)
        .bind(takeover)
        .fetch_one(pool)
        .await?;

    Ok(revision)
}

/// Optimistically move the persisted cursor forward. The write only lands when
/// the persisted state is still exactly what the caller read; otherwise a
/// concurrent operator command (takeover) selected a new head in the meantime
/// and the caller must recompute. A missing state row is created. Returns
/// `false` when the cursor was not written.
pub async fn advance_rundown_head(
    pool: &Pool<Sqlite>,
    state: &RundownState,
    head_item_id: Option<i64>,
    head_start_epoch: f64,
    drained: bool,
) -> Result<bool, ProcessError> {
    const QUERY: &str = "INSERT INTO rundown_state
        (channel_id, head_item_id, head_start_epoch, drained, takeover, revision)
        VALUES ($1, $2, $3, $4, 0, 1)
        ON CONFLICT(channel_id) DO UPDATE SET
            head_item_id = excluded.head_item_id,
            head_start_epoch = excluded.head_start_epoch,
            drained = excluded.drained,
            takeover = 0,
            revision = rundown_state.revision + 1
        WHERE rundown_state.takeover = $5
            AND rundown_state.drained = $6
            AND rundown_state.head_item_id IS $7
            AND abs(rundown_state.head_start_epoch - $8) < 0.001
        RETURNING revision";

    let revision: Option<i64> = sqlx::query_scalar(QUERY)
        .bind(state.channel_id)
        .bind(head_item_id)
        .bind(head_start_epoch)
        .bind(drained)
        .bind(state.takeover)
        .bind(state.drained)
        .bind(state.head_item_id)
        .bind(state.head_start_epoch)
        .fetch_optional(pool)
        .await?;

    Ok(revision.is_some())
}

async fn last_position(
    tx: &mut sqlx::SqliteConnection,
    channel_id: i32,
) -> Result<i64, ProcessError> {
    let position: Option<i64> =
        sqlx::query_scalar("SELECT MAX(position) FROM rundown_items WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?;

    Ok(position.unwrap_or(-1))
}

async fn item_position(
    tx: &mut sqlx::SqliteConnection,
    channel_id: i32,
    item_id: i64,
) -> Result<i64, ProcessError> {
    sqlx::query_scalar("SELECT position FROM rundown_items WHERE channel_id = $1 AND id = $2")
        .bind(channel_id)
        .bind(item_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ProcessError::Custom(format!("Rundown item {item_id} not found")))
}

/// Insert a new item after `after_position` (prepend with a negative value,
/// append at the end when `None`).
pub async fn insert_rundown_item(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    after_position: Option<i64>,
    item: &RundownItem,
) -> Result<RundownItem, ProcessError> {
    let mut tx = pool.begin().await?;

    let max = last_position(&mut tx, channel_id).await?;

    let target = match after_position {
        Some(position) => position.clamp(-1, max),
        None => max,
    };

    sqlx::query(
        "UPDATE rundown_items SET position = position + 1
        WHERE channel_id = $1 AND position > $2",
    )
    .bind(channel_id)
    .bind(target)
    .execute(&mut *tx)
    .await?;

    const INSERT: &str = "INSERT INTO rundown_items
        (channel_id, position, source, seek, out, duration, duration_audio, title, ad, custom_filter, audio)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, channel_id, position, source, seek, out, duration,
        duration_audio, title, ad, custom_filter, audio, created_at, updated_at";

    let inserted = sqlx::query_as::<_, RundownItem>(INSERT)
        .bind(channel_id)
        .bind(target + 1)
        .bind(&item.source)
        .bind(item.seek)
        .bind(item.out)
        .bind(item.duration)
        .bind(item.duration_audio)
        .bind(&item.title)
        .bind(item.ad)
        .bind(&item.custom_filter)
        .bind(&item.audio)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(inserted)
}

/// Replace the whole queue with the given items in one transaction. The
/// playback cursor is reset so the engine starts at the top of the new
/// queue; the revision bump inside `set_rundown_head` tells engine and UI
/// to reload.
pub async fn replace_rundown_queue(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    items: &[RundownItem],
) -> Result<(), ProcessError> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM rundown_items WHERE channel_id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    const INSERT: &str = "INSERT INTO rundown_items
        (channel_id, position, source, seek, out, duration, duration_audio, title, ad, custom_filter, audio)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)";

    for (index, item) in items.iter().enumerate() {
        sqlx::query(INSERT)
            .bind(channel_id)
            .bind(index as i64)
            .bind(&item.source)
            .bind(item.seek)
            .bind(item.out)
            .bind(item.duration)
            .bind(item.duration_audio)
            .bind(&item.title)
            .bind(item.ad)
            .bind(&item.custom_filter)
            .bind(&item.audio)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    set_rundown_head(pool, channel_id, None, 0.0, false, false).await?;

    Ok(())
}

pub async fn update_rundown_item(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    item_id: i64,
    item: &RundownItem,
) -> Result<RundownItem, ProcessError> {
    const UPDATE: &str = "UPDATE rundown_items
        SET source = $3, seek = $4, out = $5, duration = $6, duration_audio = $7,
            title = $8, ad = $9, custom_filter = $10, audio = $11, updated_at = unixepoch()
        WHERE channel_id = $1 AND id = $2";

    let result = sqlx::query(UPDATE)
        .bind(channel_id)
        .bind(item_id)
        .bind(&item.source)
        .bind(item.seek)
        .bind(item.out)
        .bind(item.duration)
        .bind(item.duration_audio)
        .bind(&item.title)
        .bind(item.ad)
        .bind(&item.custom_filter)
        .bind(&item.audio)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ProcessError::Custom(format!(
            "Rundown item {item_id} not found"
        )));
    }

    select_rundown_item(pool, channel_id, item_id)
        .await?
        .ok_or_else(|| ProcessError::Custom(format!("Rundown item {item_id} not found")))
}

/// Delete an item and close the position gap.
pub async fn delete_rundown_item(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    item_id: i64,
) -> Result<bool, ProcessError> {
    let mut tx = pool.begin().await?;

    let position: Option<i64> =
        sqlx::query_scalar("SELECT position FROM rundown_items WHERE channel_id = $1 AND id = $2")
            .bind(channel_id)
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await?;

    let Some(position) = position else {
        return Ok(false);
    };

    sqlx::query("DELETE FROM rundown_items WHERE channel_id = $1 AND id = $2")
        .bind(channel_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE rundown_items SET position = position - 1
        WHERE channel_id = $1 AND position > $2",
    )
    .bind(channel_id)
    .bind(position)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(true)
}

/// Move an item so it lands directly after the item that is currently at
/// `after_position` (prepend with a negative value, move to the end when
/// `None`). Positions are resolved before the move, so a vacated slot before
/// the anchor shifts the final slot accordingly.
pub async fn move_rundown_item(
    pool: &Pool<Sqlite>,
    channel_id: i32,
    item_id: i64,
    after_position: Option<i64>,
) -> Result<(), ProcessError> {
    let mut tx = pool.begin().await?;

    let current = item_position(&mut tx, channel_id, item_id).await?;
    let max = last_position(&mut tx, channel_id).await?;

    let target = after_position.map(|position| position.clamp(-1, max));

    if let Some(target) = target.filter(|target| *target != current) {
        if target > current {
            // Move later: the anchor item and everything between shift down,
            // the item lands directly after the anchor.
            sqlx::query(
                "UPDATE rundown_items SET position = position - 1
                WHERE channel_id = $1 AND position > $2 AND position <= $3",
            )
            .bind(channel_id)
            .bind(current)
            .bind(target)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "UPDATE rundown_items SET position = $3, updated_at = unixepoch()
                WHERE channel_id = $1 AND id = $2",
            )
            .bind(channel_id)
            .bind(item_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        } else {
            // Move earlier: everything between the anchor and the item shifts
            // up, the item lands right behind the anchor.
            sqlx::query(
                "UPDATE rundown_items SET position = position + 1
                WHERE channel_id = $1 AND position >= $2 AND position < $3",
            )
            .bind(channel_id)
            .bind(target + 1)
            .bind(current)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "UPDATE rundown_items SET position = $3, updated_at = unixepoch()
                WHERE channel_id = $1 AND id = $2",
            )
            .bind(channel_id)
            .bind(item_id)
            .bind(target + 1)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;
    use crate::db::handles::db_migrate;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db_migrate(&pool).await.unwrap();

        pool
    }

    fn item(source: &str) -> RundownItem {
        RundownItem {
            id: 0,
            channel_id: 1,
            position: 0,
            source: source.to_string(),
            seek: 0.0,
            out: 10.0,
            duration: 10.0,
            duration_audio: 0.0,
            title: None,
            ad: false,
            custom_filter: String::new(),
            audio: String::new(),
            created_at: 0,
            updated_at: None,
        }
    }

    async fn sources(pool: &Pool<Sqlite>) -> Vec<String> {
        select_rundown_items(pool, 1, None)
            .await
            .unwrap()
            .into_iter()
            .map(|item| item.source)
            .collect()
    }

    #[tokio::test]
    async fn insert_appends_and_shifts_positions() {
        let pool = pool().await;

        for source in ["a", "b", "c"] {
            insert_rundown_item(&pool, 1, None, &item(source))
                .await
                .unwrap();
        }

        assert_eq!(sources(&pool).await, ["a", "b", "c"]);

        // Insert after the first item.
        insert_rundown_item(&pool, 1, Some(0), &item("x"))
            .await
            .unwrap();
        assert_eq!(sources(&pool).await, ["a", "x", "b", "c"]);

        // Prepend with a negative anchor.
        insert_rundown_item(&pool, 1, Some(-1), &item("top"))
            .await
            .unwrap();
        assert_eq!(sources(&pool).await, ["top", "a", "x", "b", "c"]);
    }

    #[tokio::test]
    async fn delete_closes_the_position_gap() {
        let pool = pool().await;

        for source in ["a", "b", "c"] {
            insert_rundown_item(&pool, 1, None, &item(source))
                .await
                .unwrap();
        }

        let items = select_rundown_items(&pool, 1, None).await.unwrap();
        let id = items[1].id;

        assert!(delete_rundown_item(&pool, 1, id).await.unwrap());
        assert_eq!(sources(&pool).await, ["a", "c"]);

        let positions: Vec<i64> = select_rundown_items(&pool, 1, None)
            .await
            .unwrap()
            .into_iter()
            .map(|item| item.position)
            .collect();
        assert_eq!(positions, [0, 1]);
    }

    #[tokio::test]
    async fn move_reorders_without_gaps() {
        let pool = pool().await;

        for source in ["a", "b", "c", "d"] {
            insert_rundown_item(&pool, 1, None, &item(source))
                .await
                .unwrap();
        }

        let items = select_rundown_items(&pool, 1, None).await.unwrap();
        let id_of = |source: &str| items.iter().find(|item| item.source == source).unwrap().id;

        // Move "a" after position 2 ("c") -> b, c, a, d.
        move_rundown_item(&pool, 1, id_of("a"), Some(2))
            .await
            .unwrap();
        assert_eq!(sources(&pool).await, ["b", "c", "a", "d"]);

        // Move "d" to the top -> d, b, c, a.
        move_rundown_item(&pool, 1, id_of("d"), Some(-1))
            .await
            .unwrap();
        assert_eq!(sources(&pool).await, ["d", "b", "c", "a"]);

        // Append "d" (already last, no-op).
        move_rundown_item(&pool, 1, id_of("d"), None).await.unwrap();
        assert_eq!(sources(&pool).await, ["d", "b", "c", "a"]);

        // Move "a" to the end -> d, b, c, a stays.
        move_rundown_item(&pool, 1, id_of("a"), None).await.unwrap();
        assert_eq!(sources(&pool).await, ["d", "b", "c", "a"]);
    }

    #[tokio::test]
    async fn state_upserts_and_bumps_revision() {
        let pool = pool().await;

        assert_eq!(
            select_rundown_state(&pool, 1).await,
            RundownState {
                channel_id: 1,
                head_item_id: None,
                head_start_epoch: 0.0,
                drained: false,
                takeover: false,
                revision: 0,
            }
        );

        bump_rundown_revision(&pool, 1).await.unwrap();
        let first = set_rundown_head(&pool, 1, Some(7), 100.0, false, false)
            .await
            .unwrap();
        let second = set_rundown_head(&pool, 1, Some(8), 110.0, true, false)
            .await
            .unwrap();

        assert_eq!(first + 1, second);
        assert_eq!(
            select_rundown_state(&pool, 1).await,
            RundownState {
                channel_id: 1,
                head_item_id: Some(8),
                head_start_epoch: 110.0,
                drained: true,
                takeover: false,
                revision: second,
            }
        );
    }

    #[tokio::test]
    async fn takeover_is_stored_and_cleared_by_the_conditional_advance() {
        let pool = pool().await;

        set_rundown_head(&pool, 1, Some(3), 100.0, false, true)
            .await
            .unwrap();

        let state = select_rundown_state(&pool, 1).await;
        assert!(state.takeover);

        // The iterator consumes the takeover when it serves the item.
        assert!(
            advance_rundown_head(&pool, &state, Some(3), 100.0, false)
                .await
                .unwrap()
        );

        let state = select_rundown_state(&pool, 1).await;
        assert!(!state.takeover);
        assert_eq!(state.head_item_id, Some(3));
    }

    #[tokio::test]
    async fn conditional_advance_never_overwrites_a_new_takeover() {
        let pool = pool().await;

        set_rundown_head(&pool, 1, Some(3), 100.0, false, false)
            .await
            .unwrap();

        let stale = select_rundown_state(&pool, 1).await;

        // An operator jumps to another item while the iterator is serving.
        set_rundown_head(&pool, 1, Some(5), 200.0, false, true)
            .await
            .unwrap();

        assert!(
            !advance_rundown_head(&pool, &stale, Some(4), 105.0, false)
                .await
                .unwrap()
        );

        let state = select_rundown_state(&pool, 1).await;
        assert_eq!(state.head_item_id, Some(5));
        assert!(state.takeover);
    }

    #[tokio::test]
    async fn conditional_advance_creates_a_missing_state_row() {
        let pool = pool().await;
        let state = select_rundown_state(&pool, 1).await;

        assert!(
            advance_rundown_head(&pool, &state, Some(2), 50.0, false)
                .await
                .unwrap()
        );

        let state = select_rundown_state(&pool, 1).await;
        assert_eq!(state.head_item_id, Some(2));
        assert!(!state.takeover);
    }
}

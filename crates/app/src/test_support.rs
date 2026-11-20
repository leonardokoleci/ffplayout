use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use tokio_util::sync::CancellationToken;

use crate::{
    db::handles,
    player::controller::ChannelManager,
    utils::{config::PlayoutConfig, system::SystemStat},
};

pub(crate) async fn channel_manager() -> (ChannelManager, SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    handles::db_migrate(&pool).await.unwrap();
    let root = std::env::temp_dir().join(format!("ffplayout-manager-{}", uuid::Uuid::new_v4()));
    let playlists = root.join("playlists");
    let logs = root.join("logs");
    let public = root.join("public");

    // Configuration loading creates playlists and logs too. Keep every path
    // inside the fixture root so tests need no system directory permissions.
    sqlx::query("UPDATE channels SET playlists = $1, public = $2")
        .bind(playlists.to_string_lossy())
        .bind(public.to_string_lossy())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE config_global SET playlists = $1, logs = $2, public = $3")
        .bind(playlists.to_string_lossy())
        .bind(logs.to_string_lossy())
        .bind(public.to_string_lossy())
        .execute(&pool)
        .await
        .unwrap();
    let config = PlayoutConfig::new(&pool, 1, None).await.unwrap();
    let channel = handles::select_channel(&pool, &1).await.unwrap();
    let manager = ChannelManager::new(
        pool.clone(),
        channel,
        config,
        CancellationToken::new(),
        SystemStat::new(),
    )
    .await
    .unwrap();

    // Without a fixed media root the browser storage anchors to the current
    // directory; tests point it at the fixture root so cleanups stay scoped.
    *manager.storage.root.write().await = root;

    (manager, pool)
}

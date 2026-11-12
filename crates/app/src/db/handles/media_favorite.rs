use sqlx::SqlitePool;

use crate::utils::errors::ProcessError;

pub async fn select_media_favorites(
    pool: &SqlitePool,
    user_id: i32,
    channel_id: i32,
) -> Result<Vec<String>, ProcessError> {
    const QUERY: &str = "SELECT path FROM user_media_favorites
        WHERE user_id = $1 AND channel_id = $2
        ORDER BY created_at, id";

    let paths = sqlx::query_scalar(QUERY)
        .bind(user_id)
        .bind(channel_id)
        .fetch_all(pool)
        .await?;

    Ok(paths)
}

pub async fn insert_media_favorite(
    pool: &SqlitePool,
    user_id: i32,
    channel_id: i32,
    path: &str,
) -> Result<(), ProcessError> {
    const QUERY: &str = "INSERT OR IGNORE INTO user_media_favorites (user_id, channel_id, path)
        VALUES ($1, $2, $3)";

    sqlx::query(QUERY)
        .bind(user_id)
        .bind(channel_id)
        .bind(path)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn delete_media_favorite(
    pool: &SqlitePool,
    user_id: i32,
    channel_id: i32,
    path: &str,
) -> Result<bool, ProcessError> {
    const QUERY: &str = "DELETE FROM user_media_favorites
        WHERE user_id = $1 AND channel_id = $2 AND path = $3";

    let result = sqlx::query(QUERY)
        .bind(user_id)
        .bind(channel_id)
        .bind(path)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;
    use crate::db::handles::db_migrate;

    #[tokio::test]
    async fn favorites_are_scoped_per_user() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db_migrate(&pool).await.unwrap();

        let user_a: i32 = sqlx::query_scalar(
            "INSERT INTO auth_user (mail, username, password, role_id)
            VALUES ('a@x.y', 'a', 'x', 3) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let user_b: i32 = sqlx::query_scalar(
            "INSERT INTO auth_user (mail, username, password, role_id)
            VALUES ('b@x.y', 'b', 'x', 3) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        insert_media_favorite(&pool, user_a, 1, "/sub/")
            .await
            .unwrap();
        insert_media_favorite(&pool, user_a, 1, "/sub/")
            .await
            .unwrap();
        insert_media_favorite(&pool, user_a, 1, "/other/")
            .await
            .unwrap();
        insert_media_favorite(&pool, user_b, 1, "/sub/")
            .await
            .unwrap();

        assert_eq!(
            select_media_favorites(&pool, user_a, 1).await.unwrap(),
            vec!["/sub/", "/other/"]
        );
        assert_eq!(
            select_media_favorites(&pool, user_b, 1).await.unwrap(),
            vec!["/sub/"]
        );

        assert!(
            !delete_media_favorite(&pool, user_a, 1, "/missing/")
                .await
                .unwrap()
        );
        assert!(
            delete_media_favorite(&pool, user_a, 1, "/sub/")
                .await
                .unwrap()
        );

        assert_eq!(
            select_media_favorites(&pool, user_a, 1).await.unwrap(),
            vec!["/other/"]
        );
        assert_eq!(
            select_media_favorites(&pool, user_b, 1).await.unwrap(),
            vec!["/sub/"]
        );
    }
}

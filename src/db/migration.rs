use sqlx::SqlitePool;

/// Migrate existing users from substrukt's users table into allowthem.
/// Returns a map of old_user_id -> new_allowthem_user_id (as string).
/// Idempotent: skips users that already exist in allowthem by username.
pub async fn migrate_users_to_allowthem(
    pool: &SqlitePool,
    ath: &allowthem_core::AllowThem,
) -> eyre::Result<std::collections::HashMap<i64, String>> {
    let mut id_map = std::collections::HashMap::new();

    // Check if old users table exists
    let table_exists: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name='users'")
            .fetch_optional(pool)
            .await?;

    if table_exists.is_none() {
        tracing::info!("No old users table found, skipping migration");
        return Ok(id_map);
    }

    #[derive(sqlx::FromRow)]
    struct OldUser {
        id: i64,
        username: String,
        password_hash: String,
        role: String,
    }

    let old_users: Vec<OldUser> =
        sqlx::query_as("SELECT id, username, password_hash, role FROM users")
            .fetch_all(pool)
            .await?;

    if old_users.is_empty() {
        tracing::info!("No users to migrate");
        return Ok(id_map);
    }

    tracing::info!("Migrating {} users to allowthem", old_users.len());

    for old_user in &old_users {
        let username = allowthem_core::Username::new(old_user.username.clone());

        // Check if user already exists (idempotent)
        if let Ok(existing) = ath.db().get_user_by_username(&username).await {
            id_map.insert(old_user.id, existing.id.to_string());
            tracing::info!(
                "User {} already exists in allowthem, skipping",
                old_user.username
            );
            continue;
        }

        // Old users table has no email column — generate a placeholder
        let email_str = format!("{}@migrate.local", old_user.username);
        let email = allowthem_core::Email::new(email_str)
            .map_err(|e| eyre::eyre!("Invalid email for user {}: {e}", old_user.username))?;

        let new_user = ath
            .db()
            .create_user_with_hash(email, &old_user.password_hash, Some(username), None)
            .await
            .map_err(|e| eyre::eyre!("Failed to migrate user {}: {e}", old_user.username))?;

        // Assign role
        let role_name = allowthem_core::RoleName::new(&old_user.role);
        if let Ok(Some(role)) = ath.db().get_role_by_name(&role_name).await {
            ath.db()
                .assign_role(&new_user.id, &role.id)
                .await
                .map_err(|e| eyre::eyre!("Failed to assign role: {e}"))?;
        }

        id_map.insert(old_user.id, new_user.id.to_string());
        tracing::info!("Migrated user {} -> {}", old_user.username, new_user.id);
    }

    tracing::info!("User migration complete: {} users migrated", id_map.len());
    Ok(id_map)
}

/// After users are migrated, update schema: recreate app_access with TEXT user_id,
/// create app_tokens table, drop old auth tables. Idempotent.
/// Uses a single connection (not the pool) to avoid DDL interleaving across connections.
pub async fn finalize_schema(
    pool: &SqlitePool,
    id_map: &std::collections::HashMap<i64, String>,
) -> eyre::Result<()> {
    // Use a dedicated connection for DDL to avoid pool-level interleaving
    let mut conn = pool.acquire().await?;

    // Check if migration already done (old users table gone)
    let old_users_exist: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name='users'")
            .fetch_optional(&mut *conn)
            .await?;

    if old_users_exist.is_none() {
        return Ok(()); // Already migrated
    }

    tracing::info!("Finalizing schema migration...");

    // Drop the staging table if left over from a prior failed run
    sqlx::query("DROP TABLE IF EXISTS app_access_new")
        .execute(&mut *conn)
        .await?;

    // Recreate app_access with TEXT user_id
    sqlx::query(
        "CREATE TABLE app_access_new \
         (app_id INTEGER NOT NULL, user_id TEXT NOT NULL, PRIMARY KEY (app_id, user_id))",
    )
    .execute(&mut *conn)
    .await?;

    for (old_id, new_id) in id_map {
        sqlx::query(
            "INSERT OR IGNORE INTO app_access_new (app_id, user_id) \
             SELECT app_id, ? FROM app_access WHERE user_id = ?",
        )
        .bind(new_id)
        .bind(old_id)
        .execute(&mut *conn)
        .await?;
    }

    sqlx::query("DROP TABLE IF EXISTS app_access")
        .execute(&mut *conn)
        .await?;
    sqlx::query("ALTER TABLE app_access_new RENAME TO app_access")
        .execute(&mut *conn)
        .await?;

    // Create app_tokens table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS app_tokens \
         (api_token_id TEXT NOT NULL, app_id INTEGER NOT NULL, token_hash TEXT NOT NULL, \
         PRIMARY KEY (api_token_id, app_id))",
    )
    .execute(&mut *conn)
    .await?;

    // Drop old auth tables
    sqlx::query("DROP TABLE IF EXISTS api_tokens")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS invitations")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(&mut *conn)
        .await?;

    tracing::info!("Schema migration complete");
    Ok(())
}

/// One-time grandfather migration: mark all existing allowthem users as
/// email-verified so the new hard-block login policy does not lock them out.
/// Idempotent: guarded by a marker row in `substrukt_grandfather`.
pub async fn grandfather_email_verification(pool: &SqlitePool) -> eyre::Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS substrukt_grandfather \
         (name TEXT PRIMARY KEY, done_at TEXT NOT NULL)",
    )
    .execute(&mut *conn)
    .await?;

    let already: Option<String> =
        sqlx::query_scalar("SELECT name FROM substrukt_grandfather WHERE name = ?")
            .bind("email_verification")
            .fetch_optional(&mut *conn)
            .await?;
    if already.is_some() {
        return Ok(());
    }

    let updated =
        sqlx::query("UPDATE allowthem_users SET email_verified = 1 WHERE email_verified = 0")
            .execute(&mut *conn)
            .await?
            .rows_affected();

    sqlx::query("INSERT INTO substrukt_grandfather (name, done_at) VALUES (?, ?)")
        .bind("email_verification")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *conn)
        .await?;

    tracing::info!("Grandfathered {} existing users as email-verified", updated);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Minimal allowthem_users schema — only columns we touch.
        sqlx::query(
            "CREATE TABLE allowthem_users \
             (id TEXT PRIMARY KEY, email_verified INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn grandfather_is_idempotent() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO allowthem_users (id, email_verified) VALUES ('u1', 0)")
            .execute(&pool)
            .await
            .unwrap();

        grandfather_email_verification(&pool).await.unwrap();
        let v: i64 = sqlx::query_scalar("SELECT email_verified FROM allowthem_users WHERE id='u1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, 1);

        // Reset to 0 then run again — must be a no-op (marker present).
        sqlx::query("UPDATE allowthem_users SET email_verified = 0")
            .execute(&pool)
            .await
            .unwrap();
        grandfather_email_verification(&pool).await.unwrap();
        let v: i64 = sqlx::query_scalar("SELECT email_verified FROM allowthem_users WHERE id='u1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, 0, "second call must be a no-op");
    }
}

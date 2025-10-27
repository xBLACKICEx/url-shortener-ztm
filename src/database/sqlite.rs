//! # SQLite Database Implementation
//!
//! This module provides the SQLite implementation of the [`UrlDatabase`] trait.
//! It uses the `sqlx` crate for type-safe database operations and automatic migrations.
//!
//! ## Features
//!
//! - **Type-safe queries** - Compile-time SQL query validation
//! - **Automatic migrations** - Database schema management
//! - **Connection pooling** - Efficient database connection management
//! - **In-memory support** - For testing and development
//!
//! ## Database Schema
//!
//! The SQLite database uses a simple schema with a single `urls` table:
//!
//! ```sql
//! CREATE TABLE urls (
//!   id TEXT PRIMARY KEY,              -- Short identifier (nanoid, 6 characters)
//!   url TEXT NOT NULL                 -- Original URL
//! );
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use url_shortener_ztm_lib::DatabaseType;
//! use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
//! use url_shortener_ztm_lib::configuration::DatabaseSettings;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create database from configuration
//! let config = DatabaseSettings {
//!    r#type: DatabaseType::Sqlite,
//!     url: "database.db".to_string(),
//!     create_if_missing: true,
//!     max_connections: Some(16),
//!     min_connections: Some(4),
//! };
//! let db = SqliteUrlDatabase::from_config(&config).await?;
//!
//! // Run migrations to set up the schema
//! db.migrate().await?;
//!
//! // Use the database
//! db.insert_url("abc123", "https://example.com").await?;
//! let url = db.get_url("abc123").await?;
//! # Ok(())
//! # }
//! ```

use super::{DatabaseError, UrlDatabase};
use crate::configuration::DatabaseSettings;
use crate::models::{UpsertResult, Urls};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

const MAX_CAP: u32 = 64;
const MIN_CAP: u32 = 1;

/// SQLite implementation of the [`UrlDatabase`] trait.
///
/// This struct wraps a SQLite connection pool and provides methods for
/// storing and retrieving URLs. It handles connection management and
/// provides automatic migration capabilities.
///
/// # Thread Safety
///
/// This struct is `Send + Sync` and can be safely used across thread boundaries.
/// The underlying `SqlitePool` is designed for concurrent access.
///
/// # Examples
///
/// ```rust,no_run
/// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
/// use url_shortener_ztm_lib::configuration::DatabaseSettings;
/// use url_shortener_ztm_lib::DatabaseType;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = DatabaseSettings {
///     r#type: DatabaseType::Sqlite,
///     url: "database.db".to_string(),
///     create_if_missing: true,
///     max_connections: Some(16),
///     min_connections: Some(4),
/// };
/// let db = SqliteUrlDatabase::from_config(&config).await?;
/// # Ok(())
/// # }
/// ```
pub struct SqliteUrlDatabase {
    /// SQLite connection pool for database operations
    pool: SqlitePool,
}

impl SqliteUrlDatabase {
    /// Creates a new `SqliteUrlDatabase` with the given connection pool.
    ///
    /// # Arguments
    ///
    /// * `pool` - A configured SQLite connection pool
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
    /// use sqlx::SqlitePool;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let pool = SqlitePool::connect("sqlite:database.db").await?;
    /// let db = SqliteUrlDatabase::new(pool);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Creates a new `SqliteUrlDatabase` from configuration settings.
    ///
    /// This method sets up the database connection using the provided configuration,
    /// including creating the database file if specified and the file doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `config` - Database configuration settings
    ///
    /// # Returns
    ///
    /// Returns `Ok(SqliteUrlDatabase)` if the connection is successfully established,
    /// or `Err(DatabaseError::ConnectionError)` if connection fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use url_shortener_ztm_lib::DatabaseType;
    /// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
    /// use url_shortener_ztm_lib::configuration::DatabaseSettings;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DatabaseSettings {
    ///     r#type: DatabaseType::Sqlite,
    ///     url: "database.db".to_string(),
    ///     create_if_missing: true,
    ///     max_connections: Some(16),
    ///     min_connections: Some(4),
    /// };
    /// let db = SqliteUrlDatabase::from_config(&config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_config(config: &DatabaseSettings) -> Result<Self, DatabaseError> {
        let pool = get_connection_pool(config)
            .await
            .map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;

        Ok(Self::new(pool))
    }

    /// Runs database migrations to set up the schema.
    ///
    /// This method applies all migration files found in the `./migrations` directory.
    /// Migrations are run automatically and idempotently - running them multiple times
    /// is safe and will not cause errors.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if migrations complete successfully, or
    /// `Err(DatabaseError::MigrationError)` if migration fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use url_shortener_ztm_lib::DatabaseType;
    /// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
    /// use url_shortener_ztm_lib::configuration::DatabaseSettings;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DatabaseSettings { r#type: DatabaseType::Sqlite, url: "database.db".to_string(), create_if_missing: true, max_connections: Some(16),  min_connections: Some(4), }; let db = SqliteUrlDatabase::from_config(&config).await?;
    /// db.migrate().await?; // Set up the database schema
    /// # Ok(())
    /// # }
    /// ```
    pub async fn migrate(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl UrlDatabase for SqliteUrlDatabase {
    /// Retrieves the short ID by original URL from the SQLite database.
    async fn get_id_by_url(&self, url: &str) -> Result<Urls, DatabaseError> {
        let hash = sha256_bytes(url);

        let row = sqlx::query_as::<_, Urls>("SELECT id, code FROM urls WHERE url_hash = ? LIMIT 1")
            .bind(&hash[..]) // BLOB
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        match row {
            Some(record) => Ok(record),
            None => Err(DatabaseError::NotFound),
        }
    }
    /// Stores a URL with the given ID in the SQLite database.
    ///
    /// This implementation uses a prepared statement for type safety and
    /// handles duplicate key constraints by returning a `DatabaseError::Duplicate`.
    ///
    /// # Arguments
    ///
    /// * `id` - The short identifier for the URL
    /// * `url` - The original URL to store
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the URL was successfully stored, or an error if:
    /// - The ID already exists (`DatabaseError::Duplicate`)
    /// - A database error occurred (`DatabaseError::QueryError`)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use url_shortener_ztm_lib::DatabaseType;
    /// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
    /// use url_shortener_ztm_lib::configuration::DatabaseSettings;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DatabaseSettings { r#type: DatabaseType::Sqlite, url: "database.db".to_string(), create_if_missing: true, max_connections: Some(16),  min_connections: Some(4),}; let db = SqliteUrlDatabase::from_config(&config).await?;
    /// db.insert_url("abc123", "https://example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn insert_url(&self, code: &str, url: &str) -> Result<(UpsertResult, Urls), DatabaseError> {
        let hash = sha256_bytes(url);

        let inserted: Option<(i64,)> = sqlx::query_as(
            r#"
                INSERT INTO urls(code, url, url_hash)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(url_hash) DO NOTHING
                RETURNING id;
            "#,
        )
        .bind(code)
        .bind(url)
        .bind(&hash[..]) // BLOB
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            // `code` UNIQUE violation -> Duplicate id
            if e.to_string()
                .contains("UNIQUE constraint failed: urls.code")
            {
                DatabaseError::Duplicate
            } else {
                DatabaseError::QueryError(e.to_string())
            }
        })?;

        if let Some((id,)) = inserted {
            let urls = Urls { id, code: code.to_string() };
            let upsert_result = UpsertResult { id, created: true };
            return Ok((upsert_result, urls));
        }

        let existing_urls: Urls = sqlx::query_as(r#"SELECT id, code FROM urls WHERE url_hash = ?1 LIMIT 1"#)
            .bind(&hash[..])
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let upsert_result = UpsertResult { id: existing_urls.id, created: false };
        Ok((upsert_result, existing_urls))
    }

    /// Retrieves a URL by its short ID from the SQLite database.
    ///
    /// This implementation uses a prepared statement with optional result handling
    /// to safely retrieve URLs and handle the case where no matching record exists.
    ///
    /// # Arguments
    ///
    /// * `id` - The short identifier to look up
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` with the original URL if found, or an error if:
    /// - The ID was not found (`DatabaseError::NotFound`)
    /// - A database error occurred (`DatabaseError::QueryError`)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use url_shortener_ztm_lib::DatabaseType;
    /// use url_shortener_ztm_lib::database::{SqliteUrlDatabase, UrlDatabase};
    /// use url_shortener_ztm_lib::configuration::DatabaseSettings;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DatabaseSettings { r#type: DatabaseType::Sqlite, url: "database.db".to_string(), create_if_missing: true, max_connections: Some(16),  min_connections: Some(4),}; let db = SqliteUrlDatabase::from_config(&config).await?;
    /// let url = db.get_url("abc123").await?;
    /// println!("Original URL: {}", url);
    /// # Ok(())
    /// # }
    /// ```
    async fn get_url(&self, id: &str) -> Result<String, DatabaseError> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT url FROM all_short_codes u WHERE u.code = ? LIMIT 1;",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        match row {
            Some(record) => Ok(record.0),
            None => Err(DatabaseError::NotFound),
        }
    }

    async fn list_short_codes(
        &self,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<String>, DatabaseError> {
        let codes: Vec<String> =
            sqlx::query_scalar("SELECT code FROM all_short_codes LIMIT ? OFFSET ?")
                .bind(limit as i64)
                .bind(offset as i64)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(codes)
    }

    async fn insert_alias(&self, alias_code: &str, code_id: i64) -> Result<(), DatabaseError> {
        sqlx::query("INSERT INTO aliases (alias, target_id) VALUES (?, ?)")
            .bind(alias_code)
            .bind(code_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if e.to_string()
                    .contains("UNIQUE constraint failed: aliases.alias")
                {
                    DatabaseError::Duplicate
                } else {
                    DatabaseError::QueryError(e.to_string())
                }
            })?;
        Ok(())
    }

    async fn load_bloom_snapshot(&self, name: &str) -> Result<Option<Vec<u8>>, DatabaseError> {
        let data = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT data FROM bloom_snapshots WHERE name = ? LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(data)
    }

    async fn save_bloom_snapshot(&self, name: &str, data: &[u8]) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
                INSERT INTO bloom_snapshots (name, data, updated_at)
                VALUES (?1, ?2, CURRENT_TIMESTAMP)
                ON CONFLICT(name)
                DO UPDATE SET
                    data = excluded.data,
                    updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(name)
        .bind(data)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }
}

/// Creates a SQLite connection pool from configuration settings.
///
/// This function sets up the SQLite connection with the appropriate options,
/// including creating the database file if specified in the configuration.
///
/// # Arguments
///
/// * `config` - Database configuration settings
///
/// # Returns
///
/// Returns `Ok(SqlitePool)` if the connection pool is successfully created,
/// or `Err(sqlx::Error)` if connection setup fails.
///
/// # Examples
///
/// ```rust,no_run
/// use url_shortener_ztm_lib::DatabaseType;
/// use url_shortener_ztm_lib::database::get_connection_pool;
/// use url_shortener_ztm_lib::configuration::DatabaseSettings;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = DatabaseSettings {
///     r#type: DatabaseType::Sqlite,
///     url: "database.db".to_string(),
///     create_if_missing: true,
///     max_connections: Some(16),
///     min_connections: Some(4),
/// };
/// let pool = get_connection_pool(&config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn get_connection_pool(config: &DatabaseSettings) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(&config.connection_string())?
        .create_if_missing(config.create_if_missing)
        .foreign_keys(true);

    let cores = num_cpus::get().max(MIN_CAP as usize);
    let default_max = cores.saturating_mul(2).max(4) as u32; // minimum 4
    let mut max_conn = config.max_connections.unwrap_or(default_max);

    max_conn = max_conn.clamp(MIN_CAP, MAX_CAP);

    SqlitePoolOptions::new()
        .max_connections(max_conn)
        .connect_with(options)
        .await
}

fn sha256_bytes(s: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().into()
}

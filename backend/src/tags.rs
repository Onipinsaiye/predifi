//! Pool tagging/categorization and filtered market queries.
//!
//! `pools.category` remains the single primary classification (unchanged);
//! `pools.tags` is a free-form array creators can use for finer-grained
//! labels (e.g. a "Crypto" pool tagged `["btc", "price-prediction"]`).
//!
//! Provides query functions with explicit filters matching database indexes:
//! - status: "active", "closed", "settled" (matches idx_pools_state_*)
//! - creator: creator Stellar address (matches idx_pools_creator_state)
//! - tags: tag array overlap (matches idx_pools_tags GIN index)
//! - time: end_time_min, end_time_max (matches idx_pools_state_end_time)
//! - outcome: winning outcome on settled markets (matches idx_pools_settled_result)

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

/// A pool row for listing views, including its tags and filterable metadata.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PoolListingRow {
    pub pool_id: i64,
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
    pub total_stake: i64,
    pub end_time: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub state: String,
    pub creator: String,
    pub result: Option<String>,
}

/// Comprehensive filter criteria aligning market queries with database indexes.
#[derive(Debug, Clone, Default)]
pub struct MarketFilters<'a> {
    pub sort_by: &'a str,
    pub category: Option<&'a str>,
    pub tags: Option<&'a [String]>,
    pub status: &'a str,
    pub creator: Option<&'a str>,
    pub end_time_min: Option<DateTime<Utc>>,
    pub end_time_max: Option<DateTime<Utc>>,
    pub outcome: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

/// Filter criteria for counting markets matching the same filters as [`list_pools_with_filters`].
#[derive(Debug, Clone, Default)]
pub struct MarketCountFilters<'a> {
    pub category: Option<&'a str>,
    pub tags: Option<&'a [String]>,
    pub status: &'a str,
    pub creator: Option<&'a str>,
    pub end_time_min: Option<DateTime<Utc>>,
    pub end_time_max: Option<DateTime<Utc>>,
    pub outcome: Option<&'a str>,
}

/// Fetch pools matching explicit status, creator, tag, time, and outcome filters.
///
/// Queries are executed using bounded query plans backed by composite and partial
/// indexes (`idx_pools_state_end_time`, `idx_pools_state_created_at`, `idx_pools_state_total_stake`,
/// `idx_pools_creator_state`, `idx_pools_settled_result`, `idx_pools_tags`).
pub async fn list_pools_with_filters(
    pool: &PgPool,
    filters: &MarketFilters<'_>,
) -> Result<Vec<PoolListingRow>, sqlx::Error> {
    let order_clause = match filters.sort_by {
        "popular" => "total_stake DESC",
        "ending_soon" => "end_time ASC",
        _ => "created_at DESC",
    };

    let valid_status = match filters.status {
        "active" | "closed" | "settled" => filters.status,
        _ => "active",
    };

    // SECURITY: order_clause comes from the controlled match above only —
    // no raw user input reaches the format string.
    let sql = format!(
        r#"
        SELECT pool_id, name, category, tags, total_stake, end_time, created_at, state, creator, result
        FROM pools
        WHERE state = $1
          AND ($2::text IS NULL OR category = $2)
          AND ($3::text[] IS NULL OR tags && $3)
          AND ($4::text IS NULL OR creator = $4)
          AND ($5::timestamptz IS NULL OR end_time >= $5)
          AND ($6::timestamptz IS NULL OR end_time <= $6)
          AND ($7::text IS NULL OR result = $7)
        ORDER BY {order_clause}
        LIMIT $8 OFFSET $9
        "#
    );

    sqlx::query_as::<_, PoolListingRow>(&sql)
        .bind(valid_status)
        .bind(filters.category)
        .bind(filters.tags)
        .bind(filters.creator)
        .bind(filters.end_time_min)
        .bind(filters.end_time_max)
        .bind(filters.outcome)
        .bind(filters.limit)
        .bind(filters.offset)
        .fetch_all(pool)
        .await
}

/// Count pools matching the explicit filters in [`MarketCountFilters`].
pub async fn count_pools_with_filters(
    pool: &PgPool,
    filters: &MarketCountFilters<'_>,
) -> Result<i64, sqlx::Error> {
    let valid_status = match filters.status {
        "active" | "closed" | "settled" => filters.status,
        _ => "active",
    };

    let sql = r#"
        SELECT COUNT(*)
        FROM pools
        WHERE state = $1
          AND ($2::text IS NULL OR category = $2)
          AND ($3::text[] IS NULL OR tags && $3)
          AND ($4::text IS NULL OR creator = $4)
          AND ($5::timestamptz IS NULL OR end_time >= $5)
          AND ($6::timestamptz IS NULL OR end_time <= $6)
          AND ($7::text IS NULL OR result = $7)
    "#;

    let count: (i64,) = sqlx::query_as(sql)
        .bind(valid_status)
        .bind(filters.category)
        .bind(filters.tags)
        .bind(filters.creator)
        .bind(filters.end_time_min)
        .bind(filters.end_time_max)
        .bind(filters.outcome)
        .fetch_one(pool)
        .await?;

    Ok(count.0)
}

/// Legacy wrapper for [`list_pools_with_filters`] accepting positional arguments.
pub async fn list_pools(
    pool: &PgPool,
    sort_by: &str,
    category: Option<&str>,
    tags: Option<&[String]>,
    status: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<PoolListingRow>, sqlx::Error> {
    list_pools_with_filters(
        pool,
        &MarketFilters {
            sort_by,
            category,
            tags,
            status,
            creator: None,
            end_time_min: None,
            end_time_max: None,
            outcome: None,
            limit,
            offset,
        },
    )
    .await
}

/// Legacy wrapper for [`count_pools_with_filters`] accepting positional arguments.
pub async fn count_pools(
    pool: &PgPool,
    category: Option<&str>,
    tags: Option<&[String]>,
    status: &str,
) -> Result<i64, sqlx::Error> {
    count_pools_with_filters(
        pool,
        &MarketCountFilters {
            category,
            tags,
            status,
            creator: None,
            end_time_min: None,
            end_time_max: None,
            outcome: None,
        },
    )
    .await
}

/// Distinct tags in use across all pools, alphabetically — powers filter-UI
/// dropdowns on the frontend.
/// Returns all distinct pool tags in alphabetical order, or a database error.
pub async fn list_distinct_tags(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT unnest(tags) AS tag
        FROM pools
        ORDER BY tag
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(tag,)| tag).collect())
}

/// Replace the tag set for `pool_id`, scoped to `creator` so only the pool's
/// creator can edit its tags. Returns `false` if no matching row was updated
/// (pool not found, or `creator` doesn't own it).
/// Replaces a pool's tags when `creator` owns the pool.
///
/// Returns `true` when a row was updated and `false` when the pool is missing
/// or owned by another creator; database failures are returned unchanged.
pub async fn update_pool_tags(
    pool: &PgPool,
    pool_id: i64,
    creator: &str,
    tags: &[String],
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE pools SET tags = $1 WHERE pool_id = $2 AND creator = $3")
        .bind(tags)
        .bind(pool_id)
        .bind(creator)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_status_defaults_to_active() {
        let valid_status = match "bogus" {
            "active" | "closed" | "settled" => "bogus",
            _ => "active",
        };
        assert_eq!(valid_status, "active");
    }

    #[test]
    fn market_filters_default_values() {
        let filters = MarketFilters::default();
        assert_eq!(filters.sort_by, "");
        assert_eq!(filters.status, "");
        assert!(filters.category.is_none());
        assert!(filters.creator.is_none());
        assert!(filters.tags.is_none());
        assert!(filters.end_time_min.is_none());
        assert!(filters.end_time_max.is_none());
        assert!(filters.outcome.is_none());
        assert_eq!(filters.limit, 0);
        assert_eq!(filters.offset, 0);
    }

    #[test]
    fn market_count_filters_default_values() {
        let filters = MarketCountFilters::default();
        assert_eq!(filters.status, "");
        assert!(filters.category.is_none());
        assert!(filters.creator.is_none());
        assert!(filters.tags.is_none());
        assert!(filters.end_time_min.is_none());
        assert!(filters.end_time_max.is_none());
        assert!(filters.outcome.is_none());
    }
}

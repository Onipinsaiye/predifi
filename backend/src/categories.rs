//! Governed market category registry.
//!
//! Stable category IDs are the durable classification stored on pools.
//! Display labels and aliases may evolve, but do not rewrite an existing
//! pool's `category_id`.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct MarketCategory {
    pub category_id: String,
    pub display_label: String,
    pub status: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryError {
    UnknownCategory,
    Database,
}

/// Deterministic category ID for seeded and test fixtures. Production event
/// ingestion must use `resolve_category`, which consults the database registry.
pub fn category_id_for_label(label: &str) -> String {
    match label.trim().to_ascii_lowercase().as_str() {
        "sports" => "sports",
        "finance" => "finance",
        "crypto" => "crypto",
        "politics" => "politics",
        "entertainment" | "entertain" => "entertainment",
        "technology" | "tech" => "technology",
        "science" => "science",
        "other" => "other",
        _ => "other",
    }
    .to_string()
}

/// Resolve a category ID or alias to a stable ID.
pub async fn resolve_category(pool: &PgPool, input: &str) -> Result<MarketCategory, CategoryError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CategoryError::UnknownCategory);
    }

    let row = sqlx::query(
        r#"
        SELECT c.category_id, c.display_label, c.status
        FROM market_categories c
        LEFT JOIN market_category_aliases a ON a.category_id = c.category_id
        WHERE c.category_id = $1
           OR lower(a.alias) = lower($1)
        LIMIT 1
        "#,
    )
    .bind(trimmed)
    .fetch_optional(pool)
    .await
    .map_err(|_| CategoryError::Database)?;

    match row {
        Some(row) => {
            let category_id: String = row
                .try_get("category_id")
                .map_err(|_| CategoryError::Database)?;
            let display_label: String = row
                .try_get("display_label")
                .map_err(|_| CategoryError::Database)?;
            let status: String = row.try_get("status").map_err(|_| CategoryError::Database)?;
            let aliases = sqlx::query_scalar::<_, String>(
                "SELECT alias FROM market_category_aliases WHERE category_id = $1 ORDER BY alias",
            )
            .bind(&category_id)
            .fetch_all(pool)
            .await
            .map_err(|_| CategoryError::Database)?;
            Ok(MarketCategory {
                category_id,
                display_label,
                status,
                aliases,
            })
        }
        None => Err(CategoryError::UnknownCategory),
    }
}

/// List the category registry. Deprecated categories remain visible for
/// historic discovery and analytics.
pub async fn list_categories(pool: &PgPool) -> Result<Vec<MarketCategory>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT category_id, display_label, status
        FROM market_categories
        ORDER BY category_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut categories = Vec::with_capacity(rows.len());
    for row in rows {
        let category_id: String = row.get("category_id");
        let aliases = sqlx::query_scalar::<_, String>(
            "SELECT alias FROM market_category_aliases WHERE category_id = $1 ORDER BY alias",
        )
        .bind(&category_id)
        .fetch_all(pool)
        .await?;
        categories.push(MarketCategory {
            category_id,
            display_label: row.get("display_label"),
            status: row.get("status"),
            aliases,
        });
    }
    Ok(categories)
}

#[cfg(test)]
mod tests {
    use super::category_id_for_label;

    #[test]
    fn canonical_labels_map_to_stable_ids() {
        assert_eq!(category_id_for_label("Sports"), "sports");
        assert_eq!(category_id_for_label("Crypto"), "crypto");
        assert_eq!(category_id_for_label("Entertain"), "entertainment");
        assert_eq!(category_id_for_label("Tech"), "technology");
    }

    #[test]
    fn unknown_seed_label_is_mapped_to_other_not_a_free_form_id() {
        assert_eq!(category_id_for_label("Custom"), "other");
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EditorialCollection {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CollectionVersion {
    pub id: Uuid,
    pub collection_id: Uuid,
    pub version_number: i32,
    pub is_current: bool,
    pub editor: String,
    pub change_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CollectionItem {
    pub id: Uuid,
    pub version_id: Uuid,
    pub market_id: String,
    pub position: i32,
    pub editorial_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicCollectionView {
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub version_number: i32,
    pub editor: String,
    pub updated_at: DateTime<Utc>,
    pub items: Vec<PublicCollectionItem>,
    pub history: Vec<CollectionHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicCollectionItem {
    pub market_id: String,
    pub position: i32,
    pub editorial_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionHistoryEntry {
    pub version_number: i32,
    pub editor: String,
    pub change_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub enum EditorialCollectionError {
    NotFound,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for EditorialCollectionError {
    fn from(e: sqlx::Error) -> Self {
        EditorialCollectionError::Db(e)
    }
}

pub type Result<T> = std::result::Result<T, EditorialCollectionError>;

pub async fn create_collection(
    pool: &PgPool,
    slug: &str,
    title: &str,
    description: Option<&str>,
    editor: &str,
) -> Result<EditorialCollection> {
    let mut tx = pool.begin().await?;

    let collection = sqlx::query_as::<_, EditorialCollection>(
        r#"
        INSERT INTO editorial_collections (id, slug, title, description, created_by, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
        RETURNING id, slug, title, description, created_by, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(slug)
    .bind(title)
    .bind(description)
    .bind(editor)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO editorial_collection_versions
            (id, collection_id, version_number, is_current, editor, change_note, created_at)
        VALUES ($1, $2, 1, TRUE, $3, $4, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(collection.id)
    .bind(editor)
    .bind(Some("Initial version"))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(collection)
}

pub async fn add_version(
    pool: &PgPool,
    collection_id: Uuid,
    editor: &str,
    change_note: Option<&str>,
    items: &[(String, Option<String>)],
) -> Result<CollectionVersion> {
    let mut tx = pool.begin().await?;

    // Lock the collection row to serialize version creation.
    sqlx::query("SELECT id FROM editorial_collections WHERE id = $1 FOR UPDATE")
        .bind(collection_id)
        .fetch_one(&mut *tx)
        .await?;

    let next_version: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version_number), 0) + 1 FROM editorial_collection_versions WHERE collection_id = $1",
    )
    .bind(collection_id)
    .fetch_one(&mut *tx)
    .await?;

    // Flip current flag atomically: unset old, set new.
    sqlx::query(
        "UPDATE editorial_collection_versions SET is_current = FALSE WHERE collection_id = $1 AND is_current = TRUE",
    )
    .bind(collection_id)
    .execute(&mut *tx)
    .await?;

    let version = sqlx::query_as::<_, CollectionVersion>(
        r#"
        INSERT INTO editorial_collection_versions
            (id, collection_id, version_number, is_current, editor, change_note, created_at)
        VALUES ($1, $2, $3, TRUE, $4, $5, NOW())
        RETURNING id, collection_id, version_number, is_current, editor, change_note, created_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(collection_id)
    .bind(next_version)
    .bind(editor)
    .bind(change_note)
    .fetch_one(&mut *tx)
    .await?;

    for (position, (market_id, note)) in items.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO editorial_collection_items
                (id, version_id, market_id, position, editorial_note)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(version.id)
        .bind(market_id)
        .bind(position as i32)
        .bind(note.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("UPDATE editorial_collections SET updated_at = NOW() WHERE id = $1")
        .bind(collection_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(version)
}

pub async fn get_collection_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<EditorialCollection> {
    let collection = sqlx::query_as::<_, EditorialCollection>(
        "SELECT id, slug, title, description, created_by, created_at, updated_at FROM editorial_collections WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?
    .ok_or(EditorialCollectionError::NotFound)?;
    Ok(collection)
}

pub async fn get_version(
    pool: &PgPool,
    collection_id: Uuid,
    version_number: i32,
) -> Result<CollectionVersion> {
    let version = sqlx::query_as::<_, CollectionVersion>(
        r#"
        SELECT id, collection_id, version_number, is_current, editor, change_note, created_at
        FROM editorial_collection_versions
        WHERE collection_id = $1 AND version_number = $2
        "#,
    )
    .bind(collection_id)
    .bind(version_number)
    .fetch_optional(pool)
    .await?
    .ok_or(EditorialCollectionError::NotFound)?;
    Ok(version)
}

async fn get_current_version(
    pool: &PgPool,
    collection_id: Uuid,
) -> Result<CollectionVersion> {
    let version = sqlx::query_as::<_, CollectionVersion>(
        r#"
        SELECT id, collection_id, version_number, is_current, editor, change_note, created_at
        FROM editorial_collection_versions
        WHERE collection_id = $1 AND is_current = TRUE
        "#,
    )
    .bind(collection_id)
    .fetch_optional(pool)
    .await?
    .ok_or(EditorialCollectionError::NotFound)?;
    Ok(version)
}

async fn get_history(
    pool: &PgPool,
    collection_id: Uuid,
) -> Result<Vec<CollectionHistoryEntry>> {
    let rows = sqlx::query_as::<_, CollectionVersion>(
        r#"
        SELECT id, collection_id, version_number, is_current, editor, change_note, created_at
        FROM editorial_collection_versions
        WHERE collection_id = $1
        ORDER BY version_number DESC
        "#,
    )
    .bind(collection_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|v| CollectionHistoryEntry {
            version_number: v.version_number,
            editor: v.editor,
            change_note: v.change_note,
            created_at: v.created_at,
        })
        .collect())
}

/// Resolve items for a version, joining against pools and filtering out
/// unavailable markets while preserving editorial position order.
pub async fn resolve_items(
    pool: &PgPool,
    version_id: Uuid,
) -> Result<Vec<PublicCollectionItem>> {
    let rows = sqlx::query_as::<_, (String, i32, Option<String>)>(
        r#"
        SELECT i.market_id, i.position, i.editorial_note
        FROM editorial_collection_items i
        INNER JOIN pools p ON p.id = i.market_id
        WHERE i.version_id = $1
          AND p.is_available = TRUE
        ORDER BY i.position ASC
        "#,
    )
    .bind(version_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(market_id, position, editorial_note)| PublicCollectionItem {
            market_id,
            position,
            editorial_note,
        })
        .collect())
}

pub async fn build_public_view(
    pool: &PgPool,
    collection: &EditorialCollection,
) -> Result<PublicCollectionView> {
    let version = get_current_version(pool, collection.id).await?;
    let items = resolve_items(pool, version.id).await?;
    let history = get_history(pool, collection.id).await?;

    Ok(PublicCollectionView {
        slug: collection.slug.clone(),
        title: collection.title.clone(),
        description: collection.description.clone(),
        version_number: version.version_number,
        editor: version.editor.clone(),
        updated_at: collection.updated_at,
        items,
        history,
    })
}

pub async fn list_public_collections(pool: &PgPool) -> Result<Vec<PublicCollectionView>> {
    let collections = sqlx::query_as::<_, EditorialCollection>(
        "SELECT id, slug, title, description, created_by, created_at, updated_at FROM editorial_collections ORDER BY title ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut views = Vec::with_capacity(collections.len());
    for collection in collections {
        views.push(build_public_view(pool, &collection).await?);
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests assume a test database with the editorial_collections,
    // editorial_collection_versions, editorial_collection_items, and pools
    // tables present. They are written to be run against a live PgPool.

    async fn setup_pool() -> PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/test".to_string());
        PgPool::connect(&url).await.expect("connect test db")
    }

    async fn insert_pool(pool: &PgPool, id: &str, available: bool) {
        sqlx::query(
            "INSERT INTO pools (id, is_available) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET is_available = $2",
        )
        .bind(id)
        .bind(available)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_stable_ordering() {
        let pool = setup_pool().await;
        let slug = format!("test-order-{}", Uuid::new_v4());
        let c = create_collection(&pool, &slug, "Order Test", None, "alice")
            .await
            .unwrap();

        insert_pool(&pool, "m1", true).await;
        insert_pool(&pool, "m2", true).await;
        insert_pool(&pool, "m3", true).await;

        let items = vec![
            ("m3".to_string(), None),
            ("m1".to_string(), None),
            ("m2".to_string(), None),
        ];
        add_version(&pool, c.id, "alice", Some("reorder"), &items)
            .await
            .unwrap();

        let view = build_public_view(&pool, &c).await.unwrap();
        let ids: Vec<_> = view.items.iter().map(|i| i.market_id.clone()).collect();
        assert_eq!(ids, vec!["m3", "m1", "m2"]);
        let positions: Vec<_> = view.items.iter().map(|i| i.position).collect();
        assert_eq!(positions, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn test_revision_history() {
        let pool = setup_pool().await;
        let slug = format!("test-rev-{}", Uuid::new_v4());
        let c = create_collection(&pool, &slug, "Rev Test", None, "alice")
            .await
            .unwrap();

        insert_pool(&pool, "m1", true).await;

        let v2 = add_version(
            &pool,
            c.id,
            "bob",
            Some("second"),
            &[("m1".to_string(), None)],
        )
        .await
        .unwrap();
        assert_eq!(v2.version_number, 2);
        assert!(v2.is_current);

        let v3 = add_version(
            &pool,
            c.id,
            "carol",
            Some("third"),
            &[("m1".to_string(), None)],
        )
        .await
        .unwrap();
        assert_eq!(v3.version_number, 3);

        let view = build_public_view(&pool, &c).await.unwrap();
        assert_eq!(view.version_number, 3);
        assert_eq!(view.editor, "carol");
        assert_eq!(view.history.len(), 3);
        assert_eq!(view.history[0].version_number, 3);
        assert_eq!(view.history[2].version_number, 1);
        assert_eq!(view.history[0].editor, "carol");

        // Only one current version.
        let current_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM editorial_collection_versions WHERE collection_id = $1 AND is_current = TRUE",
        )
        .bind(c.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(current_count, 1);
    }

    #[tokio::test]
    async fn test_missing_market_filtering() {
        let pool = setup_pool().await;
        let slug = format!("test-missing-{}", Uuid::new_v4());
        let c = create_collection(&pool, &slug, "Missing Test", None, "alice")
            .await
            .unwrap();

        insert_pool(&pool, "m1", true).await;
        insert_pool(&pool, "m2", false).await;
        // m3 intentionally absent from pools table.

        let items = vec![
            ("m1".to_string(), None),
            ("m2".to_string(), None),
            ("m3".to_string(), None),
        ];
        add_version(&pool, c.id, "alice", Some("mixed"), &items)
            .await
            .unwrap();

        let view = build_public_view(&pool, &c).await.unwrap();
        let ids: Vec<_> = view.items.iter().map(|i| i.market_id.clone()).collect();
        assert_eq!(ids, vec!["m1"]);
        assert_eq!(view.items[0].position, 0);
    }
}
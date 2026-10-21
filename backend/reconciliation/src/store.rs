use crate::{Error, IndexedPool, ProjectionStore};
use sqlx::PgPool;

/// Separate SELECT-only connection; never uses the event listener's write path.
pub struct PgProjectionStore {
    pool: PgPool,
    contract_id: String,
}

impl PgProjectionStore {
    pub fn new(pool: PgPool, contract_id: String) -> Self {
        Self { pool, contract_id }
    }
}

impl ProjectionStore for PgProjectionStore {
    async fn sample(&self, after: i64, limit: usize) -> Result<Vec<IndexedPool>, Error> {
        if !(1..=100).contains(&limit) {
            return Err("sample limit must be 1..=100".into());
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query_as::<_, IndexedPool>(
            "SELECT pool_id, total_stake::text AS total_stake, state, result,
                    transaction_timestamp() AS observed_at
             FROM pools WHERE contract_id = $1 AND pool_id > $2
             ORDER BY pool_id LIMIT $3",
        )
        .bind(&self.contract_id)
        .bind(after)
        .bind(limit as i64)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }
}

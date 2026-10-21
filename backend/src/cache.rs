use redis::{aio::ConnectionManager, AsyncCommands, RedisError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default TTL fallback bound for cached market views.
///
/// Even if an invalidation is missed (e.g. a worker crash between the DB
/// commit and the cache delete), no cached market view will persist beyond
/// this bound. Keep this in sync with the documented cache contract.
pub const MARKET_VIEW_TTL: Duration = Duration::from_secs(300);

/// Indexed events that can mutate market views and therefore require cache
/// invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexedMarketEvent {
    PredictionCreated { market_id: String },
    MarketResolved { market_id: String },
    MarketCancelled { market_id: String },
    MarketMetadataUpdated { market_id: String },
}

impl IndexedMarketEvent {
    pub fn market_id(&self) -> &str {
        match self {
            IndexedMarketEvent::PredictionCreated { market_id }
            | IndexedMarketEvent::MarketResolved { market_id }
            | IndexedMarketEvent::MarketCancelled { market_id }
            | IndexedMarketEvent::MarketMetadataUpdated { market_id } => market_id,
        }
    }

    /// Map an indexed event to the exact set of cache keys it invalidates.
    ///
    /// - `PredictionCreated`: market detail (aggregates change) + list views
    ///   (volume / participant counts change).
    /// - `MarketResolved` / `MarketCancelled`: market detail + list views
    ///   (status changes affect both).
    /// - `MarketMetadataUpdated`: market detail + list views (title/tags/etc.
    ///   are surfaced in both).
    pub fn affected_keys(&self) -> Vec<String> {
        let market_id = self.market_id();
        vec![
            market_detail_key(market_id),
            market_list_key(),
        ]
    }
}

pub fn market_detail_key(market_id: &str) -> String {
    format!("market:detail:{market_id}")
}

pub fn market_list_key() -> String {
    "market:list".to_string()
}

#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
}

impl Cache {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    /// Invalidate all cache keys affected by an indexed market event.
    ///
    /// Idempotent: deleting keys that are already absent is a no-op, so
    /// duplicate or replayed events are safe and produce no errors.
    pub async fn invalidate_for_event(
        &self,
        event: &IndexedMarketEvent,
    ) -> Result<(), RedisError> {
        let keys = event.affected_keys();
        if keys.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.clone();
        let _: () = conn.del(keys).await?;
        Ok(())
    }

    /// Store a market detail view with the TTL fallback bound applied.
    pub async fn set_market_detail(
        &self,
        market_id: &str,
        value: &str,
    ) -> Result<(), RedisError> {
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(
                market_detail_key(market_id),
                value,
                MARKET_VIEW_TTL.as_secs(),
            )
            .await?;
        Ok(())
    }

    /// Store the market list view with the TTL fallback bound applied.
    pub async fn set_market_list(&self, value: &str) -> Result<(), RedisError> {
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(market_list_key(), value, MARKET_VIEW_TTL.as_secs())
            .await?;
        Ok(())
    }
}

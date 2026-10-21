use std::sync::Arc;

use redis::AsyncCommands;
use tracing::{debug, warn};

use crate::indexer::events::IndexedEvent;
use crate::indexer::state::IndexerState;

/// TTL fallback bound (in seconds) for cached market views.
///
/// Even if an invalidation is missed (e.g. worker crash between DB commit and
/// cache delete), no cached market detail or list view will persist beyond this
/// bound. Keep this in sync with the TTL used when writing the cache entries.
pub const MARKET_CACHE_TTL_SECS: u64 = 300;

/// Cache key for a single market detail view.
fn market_detail_key(market_id: &str) -> String {
    format!("market:detail:{market_id}")
}

/// Cache keys for market list views that include the given market.
fn market_list_keys(market_id: &str) -> Vec<String> {
    vec![
        "market:list:all".to_string(),
        "market:list:active".to_string(),
        format!("market:list:by-id:{market_id}"),
    ]
}

/// Map an indexed event to the cache keys it invalidates.
///
/// - Prediction created: the market detail (aggregates/odds) and list views
///   (volume/activity) change.
/// - Market resolved / cancelled: detail and list views change (status).
/// - Market metadata updated: detail and list views change (title, tags, etc.).
fn affected_cache_keys(event: &IndexedEvent) -> Vec<String> {
    match event {
        IndexedEvent::PredictionCreated { market_id, .. }
        | IndexedEvent::MarketResolved { market_id, .. }
        | IndexedEvent::MarketCancelled { market_id, .. }
        | IndexedEvent::MarketMetadataUpdated { market_id, .. } => {
            let mut keys = vec![market_detail_key(market_id)];
            keys.extend(market_list_keys(market_id));
            keys
        }
    }
}

/// Invalidate the cache keys affected by an indexed event.
///
/// Deletion is idempotent: deleting a key that does not exist is a no-op, so
/// duplicate or replayed events are safe. Errors are logged but not propagated
/// so a transient Redis failure cannot block event processing; the TTL fallback
/// (`MARKET_CACHE_TTL_SECS`) bounds staleness in that case.
async fn invalidate_market_views(state: &IndexerState, event: &IndexedEvent) {
    let keys = affected_cache_keys(event);
    if keys.is_empty() {
        return;
    }

    let mut conn = match state.redis.get_async_connection().await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "failed to acquire redis connection for cache invalidation");
            return;
        }
    };

    let deleted: Result<u64, _> = conn.del(keys.clone()).await;
    match deleted {
        Ok(count) => {
            debug!(keys = ?keys, deleted = count, "invalidated market view caches");
        }
        Err(err) => {
            warn!(error = %err, keys = ?keys, "failed to invalidate market view caches");
        }
    }
}

/// Process a single indexed event: persist it, then invalidate affected caches.
pub async fn process_event(state: Arc<IndexerState>, event: IndexedEvent) -> anyhow::Result<()> {
    state.persist_event(&event).await?;
    invalidate_market_views(&state, &event).await;
    Ok(())
}

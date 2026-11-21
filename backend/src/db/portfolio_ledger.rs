//! Normalized, cursor-paginated wallet activity feed (#309).
//!
//! Clients should not have to reimplement chain-event normalization or
//! stitch together separate pool/prediction/claim endpoint responses to
//! understand "what has this wallet done". This module exposes a single,
//! typed, cursor-paginated feed backed by `portfolio_activity_ledger` — a
//! table populated at projection time (see
//! `crate::db::event_projection::project_event_tx`) using the exact same
//! on-chain coordinates as [`crate::db::event_projection::EventKey`]
//! (`network`, `contract_id`, `ledger`, `tx_hash`, `event_index`), so every
//! item carries its canonical transaction reference rather than just an
//! opaque internal id.
//!
//! Rows are written inside the same database transaction as the dedupe
//! claim in `projected_contract_events` and the domain write, so a row here
//! is never visible before its event is fully, atomically committed —
//! there is no separate "partial batch" state to account for at read time.
//! What *can* lag is the listener's checkpoint itself (it may not have
//! advanced past the latest ledger yet, or may be missing/malformed on a
//! fresh deployment); [`get_wallet_activity`] surfaces that as
//! [`IndexingStatus`] on every page.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use super::checkpoints::{load_checkpoint, CheckpointStatus, DEFAULT_LISTENER_ID};

/// Default page size for [`get_wallet_activity`] when the caller doesn't
/// specify one.
pub const DEFAULT_ACTIVITY_PAGE_SIZE: i64 = 20;
/// Largest page size [`get_wallet_activity`] will ever return in one call.
pub const MAX_ACTIVITY_PAGE_SIZE: i64 = 100;

/// Typed wallet-activity event.
///
/// [`ActivityEvent::Unknown`] is the forward-compatible catch-all for an
/// `event_type` this build doesn't recognize (e.g. a listener upgrade
/// projected a new event type before this binary was redeployed): it is
/// always serialized, never dropped or treated as an error, so an older
/// client/server never panics or 500s on newer data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEvent {
    PoolCreated {
        pool_id: i64,
        creator: String,
    },
    PredictionPlaced {
        pool_id: i64,
        outcome: i32,
        amount: i64,
    },
    PoolResolved {
        pool_id: i64,
        winning_outcome: i32,
    },
    /// A `WinningsClaimed` / `RefundClaimed` event, recorded when the claim
    /// ingest payload carries chain coordinates (see
    /// `crate::routes::v1::ClaimPayload`).
    Claim {
        pool_id: i64,
        amount_paid: i64,
    },
    /// Any `event_type` not recognized by this build.
    Unknown {
        raw_type: String,
    },
}

/// Decode the typed event from the ledger row's loosely-typed columns.
/// Pure and DB-free so it is unit-testable (including the `Unknown`
/// fallback for a not-yet-recognized `event_type`) without a database.
fn decode_event(
    event_type: &str,
    pool_id: Option<i64>,
    counterparty: Option<&str>,
    outcome: Option<i32>,
    amount: Option<i64>,
) -> ActivityEvent {
    match event_type {
        "pool_created" => ActivityEvent::PoolCreated {
            pool_id: pool_id.unwrap_or_default(),
            creator: counterparty.unwrap_or_default().to_string(),
        },
        "prediction_placed" => ActivityEvent::PredictionPlaced {
            pool_id: pool_id.unwrap_or_default(),
            outcome: outcome.unwrap_or_default(),
            amount: amount.unwrap_or_default(),
        },
        "pool_resolved" => ActivityEvent::PoolResolved {
            pool_id: pool_id.unwrap_or_default(),
            winning_outcome: outcome.unwrap_or_default(),
        },
        "claim" => ActivityEvent::Claim {
            pool_id: pool_id.unwrap_or_default(),
            amount_paid: amount.unwrap_or_default(),
        },
        other => ActivityEvent::Unknown {
            raw_type: other.to_string(),
        },
    }
}

/// Opaque pagination cursor: the `(ledger, event_index, id)` tuple the
/// underlying query sorts and tie-breaks by, so pagination stays stable and
/// deterministic even when many events share a ledger.
///
/// Encoded as base64url so clients never parse or depend on its internal
/// shape (treated as an opaque token), without pulling in a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityCursor {
    pub ledger: i64,
    pub event_index: i32,
    pub id: i64,
}

impl ActivityCursor {
    pub fn encode(&self) -> String {
        let raw = format!("{}:{}:{}", self.ledger, self.event_index, self.id);
        base64url_encode(raw.as_bytes())
    }

    /// Decode a cursor string. Returns `None` for anything malformed —
    /// callers must treat an invalid cursor as a client error, never guess
    /// a starting point.
    pub fn decode(cursor: &str) -> Option<Self> {
        let raw = base64url_decode(cursor)?;
        let raw = String::from_utf8(raw).ok()?;
        let mut parts = raw.split(':');
        let ledger = parts.next()?.parse().ok()?;
        let event_index = parts.next()?.parse().ok()?;
        let id = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            ledger,
            event_index,
            id,
        })
    }
}

// ── Minimal base64url (no padding) ───────────────────────────────────────
//
// Hand-rolled (mirroring the existing base32 strkey codec in
// `crate::validated_types`) so the cursor can be opaque without adding a
// `base64` dependency for this one use.

const B64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64URL_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL_ALPHABET[(n & 0x3F) as usize] as char);
        }
    }
    out
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let value = B64URL_ALPHABET.iter().position(|&b| b == c)? as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push(((bits >> bit_count) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// One wallet-activity item with its canonical on-chain transaction
/// reference — not just an internal row id.
#[derive(Debug, Clone, Serialize)]
pub struct WalletActivityItem {
    pub network: String,
    pub contract_id: String,
    pub ledger: i64,
    pub tx_hash: String,
    pub event_index: i32,
    pub created_at: DateTime<Utc>,
    pub event: ActivityEvent,
    /// Opaque cursor for this item; pass as `?after=` to fetch the next page.
    pub cursor: String,
}

/// Whether a wallet-activity response reflects a fully-committed indexer
/// checkpoint, or may be behind/partial.
#[derive(Debug, Clone, Serialize)]
pub struct IndexingStatus {
    /// `true` only when the listener checkpoint is a valid, known ledger.
    /// Rows already returned are always committed (see module docs); this
    /// instead tells the caller whether there *could* be more recent chain
    /// activity not yet indexed at all.
    pub complete: bool,
    /// The committed checkpoint ledger, when known.
    pub checkpoint_ledger: Option<i64>,
    /// Human-readable reason when `complete` is `false`.
    pub reason: Option<String>,
}

/// A page of wallet activity plus pagination and indexing-status metadata.
#[derive(Debug, Clone, Serialize)]
pub struct WalletActivityPage {
    pub items: Vec<WalletActivityItem>,
    pub next_cursor: Option<String>,
    pub indexing_status: IndexingStatus,
}

#[derive(FromRow)]
struct ActivityRow {
    id: i64,
    network: String,
    contract_id: String,
    ledger: i64,
    tx_hash: String,
    event_index: i32,
    event_type: String,
    pool_id: Option<i64>,
    counterparty: Option<String>,
    outcome: Option<i32>,
    amount: Option<i64>,
    created_at: DateTime<Utc>,
}

/// Fetch one page of `wallet_address`'s normalized activity, newest-first
/// by `(ledger, event_index, id)`... actually oldest-first ascending, so
/// `next_cursor` always advances forward through chain history.
pub async fn get_wallet_activity(
    pool: &PgPool,
    wallet_address: &str,
    after: Option<ActivityCursor>,
    limit: i64,
) -> Result<WalletActivityPage, sqlx::Error> {
    let limit = limit.clamp(1, MAX_ACTIVITY_PAGE_SIZE);

    let rows = sqlx::query_as::<_, ActivityRow>(
        r#"
        SELECT id, network, contract_id, ledger, tx_hash, event_index, event_type,
               pool_id, counterparty, outcome, amount, created_at
        FROM portfolio_activity_ledger
        WHERE user_address = $1
          AND (
            $2::bigint IS NULL
            OR (ledger, event_index, id) > ($2, $3, $4)
          )
        ORDER BY ledger ASC, event_index ASC, id ASC
        LIMIT $5
        "#,
    )
    .bind(wallet_address)
    .bind(after.map(|c| c.ledger))
    .bind(after.map(|c| c.event_index))
    .bind(after.map(|c| c.id))
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let rows = if has_more {
        &rows[..limit as usize]
    } else {
        &rows[..]
    };

    let items: Vec<WalletActivityItem> = rows
        .iter()
        .map(|row| WalletActivityItem {
            network: row.network.clone(),
            contract_id: row.contract_id.clone(),
            ledger: row.ledger,
            tx_hash: row.tx_hash.clone(),
            event_index: row.event_index,
            created_at: row.created_at,
            event: decode_event(
                &row.event_type,
                row.pool_id,
                row.counterparty.as_deref(),
                row.outcome,
                row.amount,
            ),
            cursor: ActivityCursor {
                ledger: row.ledger,
                event_index: row.event_index,
                id: row.id,
            }
            .encode(),
        })
        .collect();

    let next_cursor = if has_more {
        items.last().map(|item| item.cursor.clone())
    } else {
        None
    };

    let indexing_status = match load_checkpoint(pool, DEFAULT_LISTENER_ID).await {
        CheckpointStatus::Valid(ledger) => IndexingStatus {
            complete: true,
            checkpoint_ledger: Some(ledger as i64),
            reason: None,
        },
        CheckpointStatus::Missing => IndexingStatus {
            complete: false,
            checkpoint_ledger: None,
            reason: Some("listener checkpoint not yet established".to_string()),
        },
        CheckpointStatus::Malformed(reason) => IndexingStatus {
            complete: false,
            checkpoint_ledger: None,
            reason: Some(reason),
        },
    };

    Ok(WalletActivityPage {
        items,
        next_cursor,
        indexing_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_event_type_decodes_without_panicking() {
        let event = decode_event("some_future_event", Some(1), None, None, None);
        assert_eq!(
            event,
            ActivityEvent::Unknown {
                raw_type: "some_future_event".to_string()
            }
        );
    }

    #[test]
    fn known_event_types_decode_correctly() {
        assert_eq!(
            decode_event("pool_created", Some(7), Some("GCREATOR"), None, None),
            ActivityEvent::PoolCreated {
                pool_id: 7,
                creator: "GCREATOR".to_string()
            }
        );
        assert_eq!(
            decode_event("prediction_placed", Some(7), None, Some(1), Some(500)),
            ActivityEvent::PredictionPlaced {
                pool_id: 7,
                outcome: 1,
                amount: 500
            }
        );
        assert_eq!(
            decode_event("pool_resolved", Some(7), None, Some(1), None),
            ActivityEvent::PoolResolved {
                pool_id: 7,
                winning_outcome: 1
            }
        );
        assert_eq!(
            decode_event("claim", Some(7), None, None, Some(900)),
            ActivityEvent::Claim {
                pool_id: 7,
                amount_paid: 900
            }
        );
    }

    #[test]
    fn cursor_roundtrips_through_encode_decode() {
        let cursor = ActivityCursor {
            ledger: 123_456,
            event_index: 3,
            id: 987_654_321,
        };
        let encoded = cursor.encode();
        let decoded = ActivityCursor::decode(&encoded).expect("valid cursor must decode");
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn cursor_decode_rejects_garbage() {
        assert!(ActivityCursor::decode("not-a-cursor!!").is_none());
        assert!(ActivityCursor::decode("").is_none());
    }

    #[test]
    fn cursor_is_opaque_not_plaintext() {
        let cursor = ActivityCursor {
            ledger: 42,
            event_index: 0,
            id: 1,
        };
        // The encoded form must not be the literal "42:0:1" — it is meant
        // to be treated as opaque, not parsed by clients.
        assert_ne!(cursor.encode(), "42:0:1");
    }
}

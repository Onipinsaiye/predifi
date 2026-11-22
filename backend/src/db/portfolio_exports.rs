// backend/src/db/portfolio_exports.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Documented schema version for the portfolio activity CSV export.
///
/// Bump this whenever the column set or semantics change so downstream
/// consumers can detect breaking changes.
pub const PORTFOLIO_CSV_SCHEMA_VERSION: &str = "1";

/// Stable, documented column order for the portfolio activity CSV export.
///
/// Columns:
/// - `schema_version`: version of this export schema (see [`PORTFOLIO_CSV_SCHEMA_VERSION`])
/// - `network`: network identifier the activity occurred on
/// - `event_type`: machine-readable event type (e.g. `deposit`, `withdrawal`, `trade`)
/// - `created_at`: ISO-8601 UTC timestamp of the event
/// - `asset_code`: asset code / symbol
/// - `asset_issuer`: asset issuer (empty for native assets)
/// - `amount`: exact decimal amount as a string (never a float)
/// - `transaction_ref`: on-chain or internal transaction reference
/// - `id`: stable unique identifier for the activity row
pub const PORTFOLIO_CSV_COLUMNS: &[&str] = &[
    "schema_version",
    "network",
    "event_type",
    "created_at",
    "asset_code",
    "asset_issuer",
    "amount",
    "transaction_ref",
    "id",
];

/// Default page size for cursor-based pagination of large exports.
pub const PORTFOLIO_CSV_DEFAULT_PAGE_SIZE: i64 = 1000;

/// Maximum allowed page size for cursor-based pagination of large exports.
pub const PORTFOLIO_CSV_MAX_PAGE_SIZE: i64 = 10_000;

/// A single portfolio activity row as exported to CSV.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PortfolioActivityExportRow {
    pub id: Uuid,
    pub network: String,
    pub event_type: String,
    pub created_at: DateTime<Utc>,
    pub asset_code: String,
    pub asset_issuer: Option<String>,
    /// Exact decimal amount stored as a string to preserve precision.
    pub amount: String,
    pub transaction_ref: Option<String>,
}

/// Cursor for deterministic, cursor-based pagination over
/// `(created_at, id)` ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioExportCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

/// Result of a single page of the CSV export.
#[derive(Debug, Clone)]
pub struct PortfolioCsvPage {
    /// CSV body (without header) for this page.
    pub csv: String,
    /// Cursor to fetch the next page, or `None` if this is the last page.
    pub next_cursor: Option<PortfolioExportCursor>,
}

/// Sanitize a single CSV cell to prevent formula injection.
///
/// Cells beginning with `=`, `+`, `-`, `@`, a tab (`\t`), or a carriage
/// return (`\r`) are prefixed with a single quote (`'`) so spreadsheet
/// applications treat them as literal text rather than formulas.
pub fn sanitize_csv_cell(value: &str) -> String {
    let needs_prefix = matches!(
        value.chars().next(),
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r')
    );

    if needs_prefix {
        format!("'{}", value)
    } else {
        value.to_string()
    }
}

/// Escape and quote a CSV field according to RFC 4180, after sanitizing it
/// against formula injection.
pub fn escape_csv_field(value: &str) -> String {
    let sanitized = sanitize_csv_cell(value);
    let needs_quotes = sanitized.contains(',')
        || sanitized.contains('"')
        || sanitized.contains('\n')
        || sanitized.contains('\r');

    if needs_quotes {
        let escaped = sanitized.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        sanitized
    }
}

/// Render the CSV header line for the portfolio activity export.
pub fn csv_header() -> String {
    PORTFOLIO_CSV_COLUMNS.join(",")
}

/// Render a single activity row as a CSV line (no trailing newline).
pub fn render_csv_row(row: &PortfolioActivityExportRow) -> String {
    let fields = [
        PORTFOLIO_CSV_SCHEMA_VERSION.to_string(),
        row.network.clone(),
        row.event_type.clone(),
        row.created_at.to_rfc3339(),
        row.asset_code.clone(),
        row.asset_issuer.clone().unwrap_or_default(),
        row.amount.clone(),
        row.transaction_ref.clone().unwrap_or_default(),
        row.id.to_string(),
    ];

    fields
        .iter()
        .map(|f| escape_csv_field(f))
        .collect::<Vec<_>>()
        .join(",")
}

/// Fetch a single page of portfolio activity for CSV export.
///
/// Rows are ordered deterministically by `(created_at, id)` ascending. When
/// `cursor` is provided, only rows strictly after the cursor are returned.
pub async fn fetch_portfolio_activity_page(
    pool: &PgPool,
    user_id: Uuid,
    cursor: Option<&PortfolioExportCursor>,
    limit: i64,
) -> Result<Vec<PortfolioActivityExportRow>, sqlx::Error> {
    let limit = limit.clamp(1, PORTFOLIO_CSV_MAX_PAGE_SIZE);

    let rows = match cursor {
        Some(c) => {
            sqlx::query_as::<_, PortfolioActivityExportRow>(
                r#"
                SELECT
                    id,
                    network,
                    event_type,
                    created_at,
                    asset_code,
                    asset_issuer,
                    amount::text AS amount,
                    transaction_ref
                FROM portfolio_activity
                WHERE user_id = $1
                  AND (created_at, id) > ($2, $3)
                ORDER BY created_at ASC, id ASC
                LIMIT $4
                "#,
            )
            .bind(user_id)
            .bind(c.created_at)
            .bind(c.id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, PortfolioActivityExportRow>(
                r#"
                SELECT
                    id,
                    network,
                    event_type,
                    created_at,
                    asset_code,
                    asset_issuer,
                    amount::text AS amount,
                    transaction_ref
                FROM portfolio_activity
                WHERE user_id = $1
                ORDER BY created_at ASC, id ASC
                LIMIT $2
                "#,
            )
            .bind(user_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}

/// Build a single CSV page (body only, no header) from a set of rows.
///
/// The returned `next_cursor` is `Some` when a full page was returned,
/// indicating more rows may be available.
pub fn build_csv_page(
    rows: &[PortfolioActivityExportRow],
    limit: i64,
) -> PortfolioCsvPage {
    let mut csv = String::new();
    for row in rows {
        csv.push_str(&render_csv_row(row));
        csv.push('\n');
    }

    let next_cursor = if rows.len() as i64 >= limit {
        rows.last().map(|r| PortfolioExportCursor {
            created_at: r.created_at,
            id: r.id,
        })
    } else {
        None
    };

    PortfolioCsvPage { csv, next_cursor }
}

/// Export the full portfolio activity for a user as CSV, paginating
/// internally. The returned string includes the header line.
pub async fn export_portfolio_activity_csv(
    pool: &PgPool,
    user_id: Uuid,
    page_size: i64,
) -> Result<String, sqlx::Error> {
    let page_size = page_size.clamp(1, PORTFOLIO_CSV_MAX_PAGE_SIZE);

    let mut out = String::new();
    out.push_str(&csv_header());
    out.push('\n');

    let mut cursor: Option<PortfolioExportCursor> = None;
    loop {
        let rows =
            fetch_portfolio_activity_page(pool, user_id, cursor.as_ref(), page_size).await?;

        if rows.is_empty() {
            break;
        }

        let page = build_csv_page(&rows, page_size);
        out.push_str(&page.csv);

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_row(
        id: Uuid,
        created_at: DateTime<Utc>,
        amount: &str,
        event_type: &str,
    ) -> PortfolioActivityExportRow {
        PortfolioActivityExportRow {
            id,
            network: "stellar".to_string(),
            event_type: event_type.to_string(),
            created_at,
            asset_code: "USDC".to_string(),
            asset_issuer: Some("GISSUER".to_string()),
            amount: amount.to_string(),
            transaction_ref: Some("tx-ref-1".to_string()),
        }
    }

    #[test]
    fn sanitizes_formula_injection_prefixes() {
        assert_eq!(sanitize_csv_cell("=SUM(A1:A2)"), "'=SUM(A1:A2)");
        assert_eq!(sanitize_csv_cell("+1"), "'+1");
        assert_eq!(sanitize_csv_cell("-1"), "'-1");
        assert_eq!(sanitize_csv_cell("@cmd"), "'@cmd");
        assert_eq!(sanitize_csv_cell("\tfoo"), "'\tfoo");
        assert_eq!(sanitize_csv_cell("\rfoo"), "'\rfoo");
        assert_eq!(sanitize_csv_cell("plain"), "plain");
        assert_eq!(sanitize_csv_cell(""), "");
    }

    #[test]
    fn escapes_quotes_and_commas() {
        assert_eq!(escape_csv_field("a,b"), "\"a,b\"");
        assert_eq!(escape_csv_field("he said \"hi\""), "\"he said \"\"hi\"\"\"");
        assert_eq!(escape_csv_field("line\nbreak"), "\"line\nbreak\"");
        // Formula prefix + comma => sanitized then quoted.
        assert_eq!(escape_csv_field("=a,b"), "\"'=a,b\"");
    }

    #[test]
    fn preserves_exact_decimal_amounts() {
        let id = Uuid::nil();
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let row = sample_row(id, ts, "1234567890.123456789012345678", "deposit");
        let line = render_csv_row(&row);
        assert!(line.contains("1234567890.123456789012345678"));
        // Ensure no float rounding occurred.
        assert!(!line.contains("1234567890.1234568"));
    }

    #[test]
    fn header_matches_documented_columns() {
        assert_eq!(
            csv_header(),
            "schema_version,network,event_type,created_at,asset_code,asset_issuer,amount,transaction_ref,id"
        );
    }

    #[test]
    fn row_contains_schema_version_and_iso_timestamp() {
        let id = Uuid::nil();
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let row = sample_row(id, ts, "1.0", "withdrawal");
        let line = render_csv_row(&row);
        assert!(line.starts_with(PORTFOLIO_CSV_SCHEMA_VERSION));
        assert!(line.contains("2024-01-01T00:00:00+00:00"));
        assert!(line.contains("withdrawal"));
    }

    #[test]
    fn deterministic_ordering_by_created_at_then_id() {
        let ts1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();

        let id_a = Uuid::from_u128(1);
        let id_b = Uuid::from_u128(2);

        let mut rows = vec![
            sample_row(id_b, ts1, "1", "deposit"),
            sample_row(id_a, ts2, "2", "deposit"),
            sample_row(id_a, ts1, "3", "deposit"),
        ];

        rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        assert_eq!(rows[0].id, id_a);
        assert_eq!(rows[0].created_at, ts1);
        assert_eq!(rows[1].id, id_b);
        assert_eq!(rows[1].created_at, ts1);
        assert_eq!(rows[2].id, id_a);
        assert_eq!(rows[2].created_at, ts2);
    }

    #[test]
    fn build_csv_page_sets_next_cursor_when_full() {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let id = Uuid::from_u128(42);
        let rows = vec![sample_row(id, ts, "1", "deposit")];

        let page = build_csv_page(&rows, 1);
        assert!(page.next_cursor.is_some());
        let cursor = page.next_cursor.unwrap();
        assert_eq!(cursor.id, id);
        assert_eq!(cursor.created_at, ts);

        let page_partial = build_csv_page(&rows, 10);
        assert!(page_partial.next_cursor.is_none());
    }
}
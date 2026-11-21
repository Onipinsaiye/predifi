# PrediFi Rust client

`predifi-client` is a read-only asynchronous Rust client for PrediFi's
documented HTTP API. It supports typed market, prediction, statistics,
leaderboard, referral, and portfolio queries, including automatic traversal of
the cursor-paginated market prediction endpoint.

The client does not sign transactions, connect to wallets, or accept/store
private keys. Endpoints that currently publish flexible `DataEnvelope`
payloads return `serde_json::Value`.

## Usage

```toml
[dependencies]
predifi-client = { path = "clients/rust/predifi-client" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use predifi_client::{ClientError, PaginationQuery, PoolQuery, PredifiClient};

#[tokio::main]
async fn main() -> Result<(), ClientError> {
    let client = PredifiClient::new("http://localhost:3000")?;
    let markets = client.list_pools(&PoolQuery::default()).await?;
    let portfolio = client
        .user_predictions("G...wallet-address", &PaginationQuery::default())
        .await?;

    println!("{} markets", markets.pools.len());
    println!("{} predictions", portfolio.predictions.len());
    Ok(())
}
```

Use `market_predictions_page(market_id, after, limit)` when the caller wants
manual pagination. `market_predictions(market_id, page_size)` follows each
opaque `next_cursor` and returns all prediction rows.

`ClientError` distinguishes transport and decoding failures, unauthorized and
not-found responses, rate limits (including `Retry-After`), other API errors,
server errors, and invalid repeated pagination cursors.

## Validation

Run the client tests against their in-process local HTTP backend:

```bash
cargo test --manifest-path clients/rust/predifi-client/Cargo.toml
```

Build the crate with the repository-supported stable Rust toolchain:

```bash
cargo build --manifest-path clients/rust/predifi-client/Cargo.toml
```

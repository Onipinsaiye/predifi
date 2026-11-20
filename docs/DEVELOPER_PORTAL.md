# PrediFi Developer Portal

Welcome to the PrediFi Developer Portal — your starting point for building on the PrediFi prediction market protocol. This guide provides task-oriented navigation to all the resources you need to integrate with PrediFi.

---

## Quick Navigation

| I want to...                        | Start here                                                        |
| ----------------------------------- | ----------------------------------------------------------------- |
| **Make my first prediction**        | [Quickstart Guide](./quickstart.md)                               |
| **Understand the API**              | [API Reference](./API_REFERENCE.md)                               |
| **Deploy smart contracts**          | [Contract Deployment Guide](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md) |
| **Generate client SDKs**            | [OpenAPI Client Generation](./OPENAPI_CLIENT_GENERATION.md)       |
| **Learn contract methods**          | [Contract Reference](./contract-reference.md)                     |
| **Understand prediction lifecycle** | [Prediction Lifecycle](./prediction-lifecycle.md)                 |
| **Integrate wallets**               | [Wallet Integration](#wallet-integration)                         |
| **Handle errors**                   | [Error Handling Reference](./ERROR_HANDLING_REFERENCE.md)         |
| **Check rate limits**               | [Rate Limiting](./rate-limiting.md)                               |
| **Subscribe to events**             | [Event Schemas](./EVENT_SCHEMAS.md)                               |
| **Test on testnet**                 | [Testnet Setup](#testnet)                                         |
| **Find support**                    | [Support & Community](#support)                                   |

---

## Platform Overview

PrediFi is a decentralized prediction market protocol built on **Stellar/Soroban**. It enables users to create and participate in prediction markets for any verifiable outcome.

### Key Concepts

| Concept          | Description                                                                      |
| ---------------- | -------------------------------------------------------------------------------- |
| **Pool**         | A prediction market with a specific outcome, end time, and token for staking     |
| **Prediction**   | A user's stake on a specific outcome in a pool                                   |
| **Resolution**   | The process of determining the winning outcome (operator, oracle, or price feed) |
| **Claim**        | Winners claim their proportional share of the pool after resolution              |
| **Protocol Fee** | A percentage of the pool taken by the protocol (configurable via fee tiers)      |
| **Referral**     | Users can refer others and earn a share of the protocol fee                      |

### Networks

| Network       | Status     | Soroban RPC                                 | Contract IDs                                                                            |
| ------------- | ---------- | ------------------------------------------- | --------------------------------------------------------------------------------------- |
| **Testnet**   | ✅ Active  | `https://soroban-testnet.stellar.org:443`   | [Deployed Contracts](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md#testnet-contract-addresses)   |
| **Mainnet**   | 🚧 Planned | `https://soroban.stellar.org:443`           | TBD                                                                                     |
| **Futurenet** | ✅ Active  | `https://soroban-futurenet.stellar.org:443` | [Deployed Contracts](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md#futurenet-contract-addresses) |

Examples below identify their network and the applicable frontend, API, or
contract version. The versions describe the current source snapshot, not a
published release. See the [Integration Compatibility Matrix](./INTEGRATION_COMPATIBILITY_MATRIX.md)
for supported combinations.

---

## Getting Started

### 1. Prerequisites

- **Stellar Wallet**: [Freighter](https://freighter.app/). Other wallets are not currently listed as supported.
- **Testnet XLM**: Get from [Stellar Laboratory](https://laboratory.stellar.org/#account-creator?network=test) or [Friendbot](https://friendbot.stellar.org/)
- **Node.js**: 20 LTS
- **Rust**: Current stable toolchain (for contract development)

### 2. Choose Your Integration Path

#### Frontend Application (React/Next.js)

**Example target:** Testnet · Frontend `0.1.0` · API `1.0.0`

```bash
# Run the frontend included in this repository
cd frontend
pnpm dev
```

**Key files to review:**

- [Wallet Context](../frontend/lib/context/WalletContext.tsx) — Connection state management
- [Wallet Errors](../frontend/lib/walletErrors.ts) — Error taxonomy and recovery
- [API Client](../frontend/src/lib/api.ts) — Type-safe backend API calls

#### Backend Service (Rust)

**Example target:** Testnet · API `1.0.0`

```bash
# Query the versioned PrediFi API
curl https://api.predifi.com/api/v1/pools
```

**Key files to review:**

- [Backend API Routes](../backend/src/routes/v1.rs) — REST endpoints
- [Contract Integration](../backend/src/pool_cache.rs) — Contract interaction layer
- [Event Indexer](../backend/src/worker/stellar_listener.rs) — Event processing

#### Smart Contract Development (Soroban/Rust)

**Example target:** Network-independent build · Contract crates `0.0.0`

```bash
# Build contracts
cd contract && cargo build --target wasm32-unknown-unknown --release

# Run tests
cargo test --workspace
```

**Key files to review:**

- [Main Contract](../contract/contracts/predifi-contract/src/lib.rs) — Core logic
- [Contract Reference](./contract-reference.md) — Method documentation
- [Event Schemas](./EVENT_SCHEMAS.md) — Versioned event definitions

---

## Wallet Integration

The current frontend connects through the **Freighter browser API**.

### Connection Flow

**Example target:** Testnet · Frontend `0.1.0` · Freighter browser API

```typescript
import { useWallet } from "@/lib/context/WalletContext";

function ConnectButton() {
  const { connect, address, isConnecting, error } = useWallet();

  return (
    <div>
      {address ? (
        <span>Connected: {address.slice(0, 6)}...{address.slice(-4)}</span>
      ) : (
        <button onClick={() => void connect()} disabled={isConnecting}>
          {isConnecting ? "Connecting..." : "Connect Wallet"}
        </button>
      )}
      {error && <ErrorBanner error={error} />}
    </div>
  );
}
```

### Handling Errors

All wallet errors follow a structured taxonomy:

| Code                      | Title                | User Action               |
| ------------------------- | -------------------- | ------------------------- |
| `EXTENSION_NOT_INSTALLED` | Wallet not found     | Install Freighter         |
| `NETWORK_MISMATCH`        | Wrong network        | Switch to Testnet/Mainnet |
| `USER_REJECTED`           | Connection cancelled | Approve in wallet         |
| `TRANSACTION_TIMEOUT`     | Request timed out    | Retry                     |
| `INSUFFICIENT_BALANCE`    | Insufficient funds   | Add funds                 |

See [Wallet Errors](../frontend/lib/walletErrors.ts) for full recovery actions.

### Network Requirements

PrediFi requires the network configured for the frontend. A mismatch prevents
connection; switch networks in Freighter and try again.

**Example target:** Configurable Stellar network · Frontend `0.1.0`

```typescript
// Required network (configured via NEXT_PUBLIC_STELLAR_NETWORK)
const REQUIRED_NETWORK = "TESTNET"; // or "PUBLIC", "FUTURENET"
```

---

## API Integration

### REST API

Base URL: `https://api.predifi.com/api/v1` (production) or `http://localhost:8000/api/v1` (local)

#### Public Endpoints

**Example target:** Testnet · API `1.0.0` (`/api/v1`)

```bash
# List pools
curl https://api.predifi.com/api/v1/pools

# Get pool details
curl https://api.predifi.com/api/v1/pools/1

# Get protocol stats
curl https://api.predifi.com/api/v1/stats
```

#### Authenticated Endpoints

**Example target:** Testnet · API `1.0.0` (`/api/v1`)

```bash
# User history (requires JWT)
curl -H "Authorization: Bearer <token>" \
  https://api.predifi.com/api/v1/users/GD.../history
```

**Authentication**: JWT tokens issued via wallet signature. See [API Reference](./API_REFERENCE.md#authentication).

### WebSocket Subscriptions

**Example target:** Testnet · API `1.0.0` (`/api/v1`)

```javascript
const ws = new WebSocket("wss://api.predifi.com/api/v1/ws?address=GD...");

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  switch (msg.type) {
    case "prediction_placed":
      console.log("New prediction:", msg.data);
      break;
    case "pool_resolved":
      console.log("Pool resolved:", msg.data);
      break;
  }
};
```

See [API Reference](./API_REFERENCE.md#websocket-subscriptions) for full event types.

---

## Contract Integration

### Reading Contract State

**Example target:** Testnet · Contract crates `0.0.0`

```typescript
import { Contract, rpc } from "@stellar/stellar-sdk";

const contract = new Contract(CONTRACT_ID);
const server = new rpc.Server("https://soroban-testnet.stellar.org:443");

// Get pool
const pool = await contract.call("get_pool", { pool_id: 1 });

// Get user predictions
const predictions = await contract.call("get_user_predictions", {
  user: "GD...",
  offset: 0,
  limit: 10,
});
```

### Writing to Contract

**Example target:** Testnet · Contract crates `0.0.0`

```typescript
import {
  Contract,
  Keypair,
  Networks,
  Server,
  TransactionBuilder,
  nativeToScVal,
} from "@stellar/stellar-sdk";

const server = new Server("https://horizon-testnet.stellar.org");
const contract = new Contract(CONTRACT_ID);
// Use a disposable Testnet key only; never embed a production secret.
const keypair = Keypair.fromSecret("YOUR_SECRET");
const account = await server.loadAccount(keypair.publicKey());

const tx = new TransactionBuilder(account, {
  fee: "100",
  networkPassphrase: Networks.TESTNET,
})
  .addOperation(
    contract.call("place_prediction", {
      user: keypair.publicKey(),
      pool_id: nativeToScVal(1, { type: "u64" }),
      amount: nativeToScVal(100_000_000, { type: "i128" }), // 10 tokens
      outcome: nativeToScVal(1, { type: "u32" }),
    }),
  )
  .setTimeout(30)
  .build();

tx.sign(keypair);
const result = await server.submitTransaction(tx);
```

See [Contract Reference](./contract-reference.md) for all methods and [Quickstart](./quickstart.md) for a complete example.

---

## Event Schemas

PrediFi emits versioned events for indexers and frontends. All events follow the schema in [EVENT_SCHEMAS.md](./EVENT_SCHEMAS.md).

### Core Events

| Event                   | Topics                  | Description                |
| ----------------------- | ----------------------- | -------------------------- |
| `PoolCreatedEvent`      | `["pool_created"]`      | New pool created           |
| `PredictionPlacedEvent` | `["prediction_placed"]` | User placed prediction     |
| `PoolResolvedEvent`     | `["pool_resolved"]`     | Pool resolved with outcome |
| `WinningsClaimedEvent`  | `["winnings_claimed"]`  | User claimed winnings      |
| `PoolVoidedEvent`       | `["pool_voided"]`       | Pool canceled/no-contest   |

### Indexing Events

**Example target:** Testnet · Contract crates `0.0.0` · Event Schema v1

```rust
// Soroban event filtering
let events = env.events().all();
for event in events {
    if event.topics.get(0) == "pool_created" {
        let pool_event: PoolCreatedEvent = event.data.try_into()?;
        // Process...
    }
}
```

---

## Error Handling

All PrediFi errors follow a consistent structure:

**Example target:** Testnet · API `1.0.0`

```typescript
interface PrediFiError {
  code: string;
  message: string;
  details?: Record<string, unknown>;
}
```

### Common Error Codes

| Code                   | HTTP Status | Description                       |
| ---------------------- | ----------- | --------------------------------- |
| `UNAUTHORIZED`         | 401         | Invalid or missing JWT            |
| `POOL_NOT_FOUND`       | 404         | Pool ID doesn't exist             |
| `POOL_NOT_RESOLVED`    | 400         | Cannot claim from unresolved pool |
| `ALREADY_CLAIMED`      | 409         | User already claimed winnings     |
| `INSUFFICIENT_BALANCE` | 400         | Not enough tokens for prediction  |
| `RATE_LIMITED`         | 429         | Too many requests                 |

See [Error Handling Reference](./ERROR_HANDLING_REFERENCE.md) for complete list.

---

## Rate Limiting

PrediFi implements tiered rate limiting:

| Tier      | Burst   | Sustained | Scope               |
| --------- | ------- | --------- | ------------------- |
| Light     | 120/min | 2/sec     | Public reads        |
| Read      | 60/min  | 1/sec     | Authenticated reads |
| Write     | 20/min  | 0.33/sec  | Mutations           |
| Token     | 10/min  | 0.16/sec  | Token operations    |
| WebSocket | 10/10s  | 1/sec     | WS messages         |

Responses include `RateLimit-*` headers. See [Rate Limiting](./rate-limiting.md).

---

## Testnet

### Testnet Contract Addresses

Contract IDs are deployment-specific. Use the verified addresses in the
[deployment guide](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md#testnet-contract-addresses);
do not use placeholder IDs from examples.

### Getting Testnet Funds

1. **XLM**: [Friendbot](https://friendbot.stellar.org/) or [Laboratory](https://laboratory.stellar.org/#account-creator?network=test)
2. **USDC**: [Stellar Testnet USDC Faucet](https://faucet.stellar.org/)
3. **Other tokens**: Check [Token Whitelist](./contract-reference.md#token-whitelist)

### Testnet Checklist

- [ ] Wallet connected to Testnet
- [ ] Testnet XLM for fees
- [ ] Testnet tokens for staking
- [ ] Contract addresses configured
- [ ] API endpoint set to testnet

---

## Versioning & Compatibility

PrediFi follows semantic versioning for all interfaces:

- **Contract**: Versioned in each crate's `Cargo.toml` and pinned by the release descriptor
- **API**: Versioned in URL path (`/api/v1/`)
- **Events**: Versioned in schema (`EVENT_SCHEMAS.md`)
- **Wallet**: Freighter is an external browser extension; its version is not set by this repository.

### Compatibility Matrix

See [Integration Compatibility Matrix](./INTEGRATION_COMPATIBILITY_MATRIX.md) for supported combinations of:

- Frontend version ↔ API version ↔ Contract version
- Wallet adapter version ↔ Network
- Supported token versions

---

## Support & Community

| Channel           | Purpose                               |
| ----------------- | ------------------------------------- |
| **GitHub Issues** | Bug reports, feature requests         |
| **Discord**       | Real-time developer chat              |
| **Telegram**      | Community announcements               |
| **Email**         | Security issues: security@predifi.com |

### Documentation Links

- [Full Documentation Index](./README.md)
- [API Reference](./API_REFERENCE.md)
- [Contract Reference](./contract-reference.md)
- [Event Schemas](./EVENT_SCHEMAS.md)
- [Quickstart](./quickstart.md)
- [Deployment Guide](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md)
- [Error Reference](./ERROR_HANDLING_REFERENCE.md)
- [FAQ](./faq.md)
- [Glossary](./glossary.md)

---

## Contributing

We welcome contributions! See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run tests: `cargo test` / `npm test`
5. Submit a pull request

---

## License

MIT License — see [LICENSE](../LICENSE) for details.

---

_Last updated: 2026-11-20 | Portal version: 1.0.0_

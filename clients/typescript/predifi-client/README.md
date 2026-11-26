# PrediFi TypeScript client

`@predifi/client` is a read-only, typed TypeScript client for PrediFi's
documented HTTP API. It mirrors the Rust client in `clients/rust/predifi-client`
and adds automatic offset and cursor pagination, explicit base-URL/network
configuration, and structured errors that preserve the server response.

The client does not sign transactions, connect to wallets, or accept/store
private keys. Endpoints that currently publish flexible `DataEnvelope` payloads
are typed as `unknown`.

## Install

From the repository root:

```bash
npm install ./clients/typescript/predifi-client
```

For local development:

```bash
cd clients/typescript/predifi-client
npm install
npm run build
```

## Usage

```ts
import { PredifiClient, PredifiApiError } from '@predifi/client';

const client = new PredifiClient({
  baseUrl: 'http://localhost:3000',
  network: { name: 'testnet', passphrase: 'Test SDF Network ; September 2015' },
  apiVersion: 'v1',
});

const markets = await client.pools.list({ status: 'open', limit: 20 });
console.log(markets.total);

for await (const pool of client.pools.iterate({ status: 'open' })) {
  console.log(pool.pool_id, pool.name);
}

try {
  await client.pools.get(1);
} catch (error) {
  if (error instanceof PredifiApiError) {
    console.error(error.status, error.code, error.message, error.body);
  }
}
```

`baseUrl` must be an absolute `http(s)` URL and may include a path prefix.
`network` is carried for callers that also talk to Horizon or Soroban RPC; it
does not change the HTTP requests. Every request targets `<baseUrl>/api/v1/...`,
except `rootHealth()`, which targets `<baseUrl>/health`.

## Pagination

- `client.pools.iterate(query)` follows offset pagination using `total`, `limit`,
  and `offset`, yielding one pool at a time.
- `client.markets.predictionsPage(marketId, { after, limit })` fetches a single
  cursor page.
- `client.markets.predictions(marketId, { pageSize })` follows `next_cursor` and
  raises `PredifiPaginationError` if the server repeats a cursor.

## Errors

Every failure extends `PredifiError`:

| Error | Meaning |
| --- | --- |
| `PredifiValidationError` | Invalid base URL or caller option (`field`). |
| `PredifiNetworkError` | Transport failure; the original error is in `cause`. |
| `PredifiApiError` | Non-success response or error envelope. Preserves `status`, `code`, `requestId`, `retryable`, `details`, `body`, `rawBody`, and `headers`. |
| `PredifiVersionError` | 406 response or unsupported/sunset version. Adds `requested` and `supported`. |
| `PredifiPaginationError` | Cursor/offset sequence cannot advance. |

Server responses are never discarded: non-JSON bodies are kept verbatim in
`rawBody`, and parsed JSON is kept in `body`.

## Version negotiation

The client sends `Accept-Version` when `apiVersion` is configured. A 406
response, or an `UNSUPPORTED_API_VERSION` error code, raises
`PredifiVersionError`. `client.lastApiVersion` records the `X-API-Version`
response header, and `client.assertVersionSupported()` checks the API index for
sunset versions.

## Development

```bash
npm install
npm test                 # build + node:test suite
npm run typecheck
npm run examples:typecheck
```

`npm run examples:typecheck` compiles `examples/quickstart.ts` against the
generated `dist/index.d.ts`, so the published types are validated.

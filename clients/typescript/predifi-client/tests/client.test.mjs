import test from 'node:test';
import assert from 'node:assert/strict';

import {
  PredifiApiError,
  PredifiClient,
  PredifiNetworkError,
  PredifiPaginationError,
  PredifiValidationError,
  PredifiVersionError,
} from '../dist/index.js';

function pool(poolId) {
  return {
    pool_id: poolId,
    name: `Pool ${poolId}`,
    category: 'general',
    total_stake: 0,
    end_time: 1700000000,
    created_at: '2026-01-01T00:00:00Z',
    state: 'open',
    creator: 'GABC',
  };
}

function prediction(id) {
  return {
    id,
    pool_id: 9,
    user_address: 'GUSER',
    outcome: 1,
    amount: 5,
    created_at: '2026-01-01T00:00:00Z',
  };
}

function jsonResponse(body, { status = 200, headers = {} } = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

function textResponse(body, { status = 200, headers = {} } = {}) {
  return new Response(body, { status, headers });
}

function recordingFetch(handler) {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    calls.push({ url: String(url), init });
    return handler(String(url), init);
  };
  return { calls, fetchImpl };
}

test('pools.list builds the versioned URL, serializes arrays, and unwraps the success envelope', async () => {
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({
      status: 'success',
      data: {
        pools: [pool(1)],
        total: 1,
        limit: 2,
        offset: 0,
        status: 'open',
        sort_by: 'end_time',
      },
    }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test/root/', fetch: fetchImpl });

  const page = await client.pools.list({ status: 'open', limit: 2, offset: 0, tags: ['a', 'b'] });

  assert.equal(page.total, 1);
  assert.equal(page.pools[0].name, 'Pool 1');
  assert.equal(calls.length, 1);
  const url = new URL(calls[0].url);
  assert.equal(url.origin, 'https://api.example.test');
  assert.equal(url.pathname, '/root/api/v1/pools');
  assert.equal(url.searchParams.get('status'), 'open');
  assert.equal(url.searchParams.get('limit'), '2');
  assert.equal(url.searchParams.get('offset'), '0');
  assert.equal(url.searchParams.get('tags'), 'a,b');
});

test('pools.get returns flattened pool details with odds', async () => {
  const { fetchImpl } = recordingFetch(() =>
    jsonResponse({
      status: 'success',
      data: { ...pool(7), odds: [{ outcome: 1, stake: 40, odds: 0.4 }] },
    }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  const result = await client.pools.get(7);

  assert.equal(result.pool_id, 7);
  assert.equal(result.odds[0].outcome, 1);
});

test('pools.iterate follows offset pagination until total is reached', async () => {
  const pages = [
    { pools: [pool(1), pool(2)], total: 3, limit: 2, offset: 0, status: 'open', sort_by: 'created_at' },
    { pools: [pool(3)], total: 3, limit: 2, offset: 2, status: 'open', sort_by: 'created_at' },
  ];
  let index = 0;
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({ status: 'success', data: pages[index++] }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  const ids = [];
  for await (const item of client.pools.iterate({ limit: 2 })) {
    ids.push(item.pool_id);
  }

  assert.deepEqual(ids, [1, 2, 3]);
  assert.equal(calls.length, 2);
  assert.equal(new URL(calls[1].url).searchParams.get('offset'), '2');
});

test('markets.predictions follows next_cursor across pages', async () => {
  const pages = [
    { market_id: 9, predictions: [prediction(1)], limit: 1, next_cursor: 'c1' },
    { market_id: 9, predictions: [prediction(2)], limit: 1, next_cursor: null },
  ];
  let index = 0;
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({ status: 'success', data: pages[index++] }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  const ids = [];
  for await (const item of client.markets.predictions(9, { pageSize: 1 })) {
    ids.push(item.id);
  }

  assert.deepEqual(ids, [1, 2]);
  assert.equal(new URL(calls[0].url).searchParams.get('limit'), '1');
  assert.equal(new URL(calls[1].url).searchParams.get('after'), 'c1');
});

test('markets.predictions raises PredifiPaginationError when a cursor repeats', async () => {
  const { fetchImpl } = recordingFetch(() =>
    jsonResponse({
      status: 'success',
      data: { market_id: 9, predictions: [prediction(1)], limit: 1, next_cursor: 'same' },
    }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  await assert.rejects(
    async () => {
      for await (const _item of client.markets.predictions(9, { pageSize: 1 })) {
        // drain the generator
      }
    },
    (error) => {
      assert.ok(error instanceof PredifiPaginationError);
      assert.equal(error.cursor, 'same');
      return true;
    },
  );
});

test('a 400 validation error preserves the server body and details', async () => {
  const body = {
    status: 'error',
    error: {
      code: 'VALIDATION_ERROR',
      message: 'limit must be <= 100',
      request_id: 'req-1',
      retryable: false,
      details: { field: 'limit' },
    },
  };
  const { fetchImpl } = recordingFetch(() => jsonResponse(body, { status: 400 }));
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  await assert.rejects(
    () => client.pools.list({ limit: 1000 }),
    (error) => {
      assert.ok(error instanceof PredifiApiError);
      assert.equal(error.status, 400);
      assert.equal(error.code, 'VALIDATION_ERROR');
      assert.equal(error.requestId, 'req-1');
      assert.equal(error.retryable, false);
      assert.deepEqual(error.details, { field: 'limit' });
      assert.deepEqual(error.body, body);
      return true;
    },
  );
});

test('a 406 response surfaces a PredifiVersionError', async () => {
  const { fetchImpl } = recordingFetch(() =>
    jsonResponse(
      {
        status: 'error',
        error: { code: 'UNSUPPORTED_API_VERSION', message: 'v3 is not supported' },
      },
      { status: 406 },
    ),
  );
  const client = new PredifiClient({
    baseUrl: 'https://api.example.test',
    apiVersion: 'v3',
    fetch: fetchImpl,
  });

  await assert.rejects(
    () => client.health(),
    (error) => {
      assert.ok(error instanceof PredifiVersionError);
      assert.equal(error.status, 406);
      assert.equal(error.requested, 'v3');
      return true;
    },
  );
});

test('the client records the negotiated version and sends Accept-Version', async () => {
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse(
      { status: 'success', data: { status: 'ok', version: 'v1' } },
      { headers: { 'x-api-version': 'v1' } },
    ),
  );
  const client = new PredifiClient({
    baseUrl: 'https://api.example.test',
    apiVersion: 'v1',
    fetch: fetchImpl,
  });

  await client.health();

  assert.equal(client.lastApiVersion, 'v1');
  assert.equal(calls[0].init.headers.get('accept-version'), 'v1');
});

test('assertVersionSupported rejects a sunset version', async () => {
  const { fetchImpl } = recordingFetch(() =>
    jsonResponse({
      status: 'success',
      data: {
        name: 'predifi',
        version: 'v2',
        status: 'active',
        versions: [
          { version: 'v1', status: 'sunset' },
          { version: 'v2', status: 'active' },
        ],
      },
    }),
  );
  const client = new PredifiClient({
    baseUrl: 'https://api.example.test',
    apiVersion: 'v1',
    fetch: fetchImpl,
  });

  await assert.rejects(
    () => client.assertVersionSupported(),
    (error) => {
      assert.ok(error instanceof PredifiVersionError);
      assert.equal(error.requested, 'v1');
      assert.deepEqual(error.supported, ['v2']);
      return true;
    },
  );
});

test('transport failures become PredifiNetworkError with the original cause', async () => {
  const cause = new Error('ECONNREFUSED');
  const client = new PredifiClient({
    baseUrl: 'https://api.example.test',
    fetch: async () => {
      throw cause;
    },
  });

  await assert.rejects(
    () => client.health(),
    (error) => {
      assert.ok(error instanceof PredifiNetworkError);
      assert.equal(error.cause, cause);
      return true;
    },
  );
});

test('explicit base URL and network config are preserved', async () => {
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({ status: 'success', data: { status: 'ok' } }),
  );
  const client = new PredifiClient({
    baseUrl: 'https://rpc.example.test/v1/',
    network: { name: 'testnet', passphrase: 'Test SDF Network ; September 2015' },
    fetch: fetchImpl,
  });

  await client.rootHealth();

  assert.equal(new URL(calls[0].url).pathname, '/v1/health');
  assert.equal(client.network.name, 'testnet');
  assert.equal(client.network.passphrase, 'Test SDF Network ; September 2015');
});

test('an unsupported base URL protocol is rejected', () => {
  assert.throws(
    () => new PredifiClient({ baseUrl: 'ftp://api.example.test' }),
    (error) => {
      assert.ok(error instanceof PredifiValidationError);
      assert.equal(error.field, 'baseUrl');
      return true;
    },
  );
});

test('a non-JSON error body is preserved verbatim', async () => {
  const { fetchImpl } = recordingFetch(() =>
    textResponse('upstream unavailable', { status: 502, headers: { 'content-type': 'text/plain' } }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  await assert.rejects(
    () => client.health(),
    (error) => {
      assert.ok(error instanceof PredifiApiError);
      assert.equal(error.status, 502);
      assert.equal(error.rawBody, 'upstream unavailable');
      assert.match(error.message, /upstream unavailable/);
      return true;
    },
  );
});

test('bare JSON payloads are returned without an envelope', async () => {
  const { fetchImpl } = recordingFetch(() =>
    jsonResponse({ treasury_fee_bps: 10, referral_fee_bps: 5 }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  const fees = await client.fees();

  assert.equal(fees.treasury_fee_bps, 10);
  assert.equal(fees.referral_fee_bps, 5);
});

test('query serialization skips empty values and encodes booleans', async () => {
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({ status: 'success', data: { notifications: [] } }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  await client.notifications('GUSER', { unread_only: false, limit: 5, offset: undefined });

  const url = new URL(calls[0].url);
  assert.equal(url.pathname, '/api/v1/notifications/GUSER');
  assert.equal(url.searchParams.get('unread_only'), 'false');
  assert.equal(url.searchParams.get('limit'), '5');
  assert.equal(url.searchParams.has('offset'), false);
});

test('apiIndex hits the versioned root', async () => {
  const { calls, fetchImpl } = recordingFetch(() =>
    jsonResponse({ status: 'success', data: { name: 'predifi', version: 'v1', status: 'active' } }),
  );
  const client = new PredifiClient({ baseUrl: 'https://api.example.test', fetch: fetchImpl });

  const index = await client.apiIndex();

  assert.equal(index.name, 'predifi');
  assert.equal(new URL(calls[0].url).pathname, '/api/v1/');
});

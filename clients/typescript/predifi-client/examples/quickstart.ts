import {
  PredifiApiError,
  PredifiClient,
  PredifiPaginationError,
  PredifiVersionError,
  type Pool,
} from '@predifi/client';

async function main(): Promise<void> {
  const client = new PredifiClient({
    baseUrl: 'http://localhost:3000',
    network: { name: 'local' },
    apiVersion: 'v1',
  });

  const index = await client.assertVersionSupported();
  console.log(`API ${index.version} is ${index.status}`);

  const firstPage = await client.pools.list({ status: 'open', limit: 10 });
  console.log(`${firstPage.total} open pools`);

  const names: string[] = [];
  for await (const pool of client.pools.iterate({ status: 'open', limit: 10 })) {
    names.push(pool.name);
    if (names.length >= 25) {
      break;
    }
  }
  console.log(names.join(', '));

  const pool: Pool = firstPage.pools[0] ?? (await client.pools.get(1));
  console.log(pool.pool_id, pool.state);

  for await (const prediction of client.markets.predictions(pool.pool_id, { pageSize: 50 })) {
    console.log(prediction.user_address, prediction.amount);
  }

  try {
    await client.pools.get(0);
  } catch (error) {
    if (error instanceof PredifiVersionError) {
      console.error('unsupported API version', error.requested, error.supported);
    } else if (error instanceof PredifiPaginationError) {
      console.error('pagination did not advance', error.cursor);
    } else if (error instanceof PredifiApiError) {
      console.error(error.status, error.code, error.message, error.body);
    } else {
      throw error;
    }
  }
}

void main();

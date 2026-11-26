import { PredifiPaginationError, PredifiVersionError } from './errors.js';
import { PredifiHttp, type PredifiHttpOptions } from './http.js';
import type {
  ApiIndex,
  CreatorStats,
  FeeInfo,
  HealthResponse,
  LeaderboardQuery,
  LeaderboardResponse,
  MarketPrediction,
  MarketPredictionsPage,
  NotificationQuery,
  PaginationQuery,
  Pool,
  PoolList,
  PoolQuery,
  PoolWithOdds,
  PredictionHistory,
  PricesResponse,
  ProtocolStats,
  ReferralEarnings,
  StatsQuery,
  UserInterests,
  UserPredictions,
  UserProfile,
} from './types.js';

export interface PredifiNetworkConfig {
  /** Human-readable network name, e.g. `testnet` or `mainnet`. */
  name: string;
  /** Stellar network passphrase, when callers need it alongside API calls. */
  passphrase?: string;
  /** Horizon endpoint associated with the network. */
  horizonUrl?: string;
  /** Soroban RPC endpoint associated with the network. */
  rpcUrl?: string;
}

export interface PredifiClientOptions extends Omit<PredifiHttpOptions, 'baseUrl'> {
  baseUrl: string;
  network?: PredifiNetworkConfig;
}

function asQuery(query: object): Record<string, unknown> {
  return query as Record<string, unknown>;
}

export class PoolsResource {
  constructor(private readonly http: PredifiHttp) {}

  /** Fetch one page of markets. */
  list(query: PoolQuery = {}): Promise<PoolList> {
    return this.http.get<PoolList>('pools', { query: asQuery(query) });
  }

  /** Fetch one market together with its outcome odds. */
  get(poolId: number | string): Promise<PoolWithOdds> {
    return this.http.get<PoolWithOdds>(`pools/${encodeURIComponent(String(poolId))}`);
  }

  /**
   * Walk offset pagination until `total` is reached or a short page is seen.
   *
   * The generator yields individual pools; it never buffers a full result set.
   */
  async *iterate(query: PoolQuery = {}): AsyncGenerator<Pool, void, void> {
    const limit = query.limit ?? 50;
    let offset = query.offset ?? 0;

    while (true) {
      const page = await this.list({ ...query, limit, offset });
      if (page.pools.length === 0) {
        return;
      }

      for (const pool of page.pools) {
        yield pool;
      }

      const nextOffset = offset + page.pools.length;
      if (page.pools.length < limit || nextOffset >= page.total) {
        return;
      }
      if (nextOffset <= offset) {
        throw new PredifiPaginationError('offset pagination did not advance', String(offset));
      }
      offset = nextOffset;
    }
  }
}

export class MarketsResource {
  constructor(private readonly http: PredifiHttp) {}

  /** Fetch one cursor-paginated page of predictions for a market. */
  predictionsPage(
    marketId: number | string,
    options: { after?: string; limit?: number } = {},
  ): Promise<MarketPredictionsPage> {
    return this.http.get<MarketPredictionsPage>(
      `markets/${encodeURIComponent(String(marketId))}/predictions`,
      { query: asQuery(options) },
    );
  }

  /**
   * Follow `next_cursor` until the server stops returning one.
   *
   * A repeated cursor raises `PredifiPaginationError` instead of looping.
   */
  async *predictions(
    marketId: number | string,
    options: { pageSize?: number } = {},
  ): AsyncGenerator<MarketPrediction, void, void> {
    const pageSize = options.pageSize ?? 20;
    const seenCursors = new Set<string>();
    let after: string | undefined;

    while (true) {
      const page = await this.predictionsPage(marketId, { after, limit: pageSize });
      for (const prediction of page.predictions) {
        yield prediction;
      }

      const nextCursor = page.next_cursor ?? undefined;
      if (!nextCursor) {
        return;
      }
      if (seenCursors.has(nextCursor)) {
        throw new PredifiPaginationError(
          `market predictions pagination repeated cursor ${nextCursor}`,
          nextCursor,
        );
      }
      seenCursors.add(nextCursor);
      after = nextCursor;
    }
  }
}

export class UsersResource {
  constructor(private readonly http: PredifiHttp) {}

  history(address: string, query: PaginationQuery = {}): Promise<PredictionHistory> {
    return this.http.get<PredictionHistory>(`users/${encodeURIComponent(address)}/history`, {
      query: asQuery(query),
    });
  }

  predictions(address: string, query: PaginationQuery = {}): Promise<UserPredictions> {
    return this.http.get<UserPredictions>(`users/${encodeURIComponent(address)}/predictions`, {
      query: asQuery(query),
    });
  }

  profile(address: string): Promise<UserProfile> {
    return this.http.get<UserProfile>(`users/${encodeURIComponent(address)}/profile`);
  }

  interests(address: string): Promise<UserInterests> {
    return this.http.get<UserInterests>(`users/${encodeURIComponent(address)}/interests`);
  }

  referrals(address: string): Promise<ReferralEarnings> {
    return this.http.get<ReferralEarnings>(`users/${encodeURIComponent(address)}/referrals`);
  }
}

export class CreatorsResource {
  constructor(private readonly http: PredifiHttp) {}

  stats(address: string): Promise<CreatorStats> {
    return this.http.get<CreatorStats>(`creators/${encodeURIComponent(address)}/stats`);
  }
}

/**
 * Read-only typed client for the PrediFi HTTP API.
 *
 * The client never signs transactions, connects to wallets, or stores private
 * keys. Endpoints that currently publish flexible payloads return `unknown`.
 */
export class PredifiClient {
  readonly baseUrl: string;
  readonly network: PredifiNetworkConfig;
  readonly pools: PoolsResource;
  readonly markets: MarketsResource;
  readonly users: UsersResource;
  readonly creators: CreatorsResource;

  private readonly http: PredifiHttp;

  constructor(options: PredifiClientOptions) {
    this.http = new PredifiHttp(options);
    this.baseUrl = this.http.baseUrl.toString();
    this.network = options.network ?? { name: 'custom' };
    this.pools = new PoolsResource(this.http);
    this.markets = new MarketsResource(this.http);
    this.users = new UsersResource(this.http);
    this.creators = new CreatorsResource(this.http);
  }

  /** Configured API version, if any. */
  get apiVersion(): string | undefined {
    return this.http.apiVersion;
  }

  /** API version advertised by the most recent response. */
  get lastApiVersion(): string | undefined {
    return this.http.lastApiVersion;
  }

  /** Fetch API version lifecycle information. */
  apiIndex(): Promise<ApiIndex> {
    return this.http.get<ApiIndex>('');
  }

  /** Fetch backend health and dependency status. */
  health(): Promise<HealthResponse> {
    return this.http.get<HealthResponse>('health');
  }

  /** Fetch the legacy root health endpoint (outside `/api/v1`). */
  rootHealth(): Promise<HealthResponse> {
    return this.http.get<HealthResponse>('health', { apiPrefix: false });
  }

  /** Fetch protocol-wide aggregate statistics. */
  stats(query: StatsQuery = {}): Promise<ProtocolStats> {
    return this.http.get<ProtocolStats>('stats', { query: asQuery(query) });
  }

  /** Fetch the protocol fee configuration. */
  fees(): Promise<FeeInfo> {
    return this.http.get<FeeInfo>('fees');
  }

  /** Fetch the latest cached asset prices. */
  prices(): Promise<PricesResponse> {
    return this.http.get<PricesResponse>('prices');
  }

  /** Fetch the protocol leaderboard. */
  leaderboard(query: LeaderboardQuery = {}): Promise<LeaderboardResponse> {
    return this.http.get<LeaderboardResponse>('leaderboard', { query: asQuery(query) });
  }

  /** Fetch referral earnings for an address. */
  referrals(address: string): Promise<ReferralEarnings> {
    return this.http.get<ReferralEarnings>(`referrals/${encodeURIComponent(address)}`);
  }

  /** Fetch the tags attached to a market. */
  poolTags(poolId: number | string): Promise<unknown> {
    return this.http.get<unknown>(`pools/${encodeURIComponent(String(poolId))}/tags`);
  }

  /** Fetch the available market tags. */
  tags(): Promise<unknown> {
    return this.http.get<unknown>('tags');
  }

  /** Fetch a per-market leaderboard. */
  poolLeaderboard(poolId: number | string, query: LeaderboardQuery = {}): Promise<unknown> {
    return this.http.get<unknown>(`pools/${encodeURIComponent(String(poolId))}/leaderboard`, {
      query: asQuery(query),
    });
  }

  /** Fetch referral estimate data for an address. */
  referralEstimate(address: string): Promise<unknown> {
    return this.http.get<unknown>(`referrals/${encodeURIComponent(address)}/estimate`);
  }

  /** Fetch notifications for an address (flexible payload). */
  notifications(address: string, query: NotificationQuery = {}): Promise<unknown> {
    return this.http.get<unknown>(`notifications/${encodeURIComponent(address)}`, {
      query: asQuery(query),
    });
  }

  /**
   * Verify that `version` is listed by the server and not sunset.
   *
   * Defaults to the configured `apiVersion`. The index document is returned so
   * callers can inspect lifecycle metadata.
   */
  async assertVersionSupported(version: string | undefined = this.http.apiVersion): Promise<ApiIndex> {
    const index = await this.apiIndex();
    if (!version) {
      return index;
    }

    const versions = index.versions ?? [];
    const match = versions.find((candidate) => candidate.version === version);
    if (!match) {
      throw new PredifiVersionError(`API version ${version} is not listed by the server`, {
        requested: version,
        supported: versions.map((candidate) => candidate.version),
        body: index,
      });
    }
    if (match.status === 'sunset') {
      throw new PredifiVersionError(`API version ${version} has been sunset`, {
        status: 410,
        requested: version,
        supported: versions
          .filter((candidate) => candidate.status !== 'sunset')
          .map((candidate) => candidate.version),
        body: index,
      });
    }

    return index;
  }
}

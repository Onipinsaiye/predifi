/**
 * Wire types for the PrediFi HTTP API.
 *
 * These mirror the JSON produced by `backend/openapi.json` and consumed by the
 * Rust client in `clients/rust/predifi-client`. Fields marked nullable are
 * omitted or `null` depending on the endpoint.
 */

export type ApiTimestamp = number | string;

export interface ApiVersion {
  version: string;
  status: string;
  deprecated_at?: string | null;
  sunset_at?: string | null;
  successor?: string | null;
}

export interface ApiIndex {
  name: string;
  version: string;
  status: string;
  versions?: ApiVersion[];
  deprecated_routes?: unknown[];
  deprecation_policy?: string | null;
}

export interface HealthResponse {
  status: string;
  version?: string | null;
  service?: string | null;
  dependencies?: unknown;
  errors?: unknown;
}

export interface Pool {
  pool_id: number;
  name: string;
  category: string;
  total_stake: number;
  end_time: ApiTimestamp;
  created_at: string;
  state: string;
  creator: string;
  token?: string | null;
  result?: string | null;
  tags?: string[] | null;
}

export interface PoolList {
  pools: Pool[];
  total: number;
  limit: number;
  offset: number;
  status: string;
  category?: string | null;
  sort_by: string;
}

export interface OutcomeOdds {
  outcome: number;
  stake: number;
  odds: number;
}

export interface PoolWithOdds extends Pool {
  odds: OutcomeOdds[];
}

export interface PoolQuery {
  sort_by?: string;
  category?: string;
  status?: string;
  creator?: string;
  tags?: string | string[];
  tag?: string;
  end_time_min?: string;
  end_time_max?: string;
  from_time?: string;
  to_time?: string;
  outcome?: string;
  limit?: number;
  offset?: number;
}

export interface StatsQuery {
  category?: string;
  status?: string;
}

export interface PaginationQuery {
  limit?: number;
  offset?: number;
}

export interface LeaderboardQuery {
  rank_by?: string;
  limit?: number;
  offset?: number;
  period?: string;
}

export interface NotificationQuery {
  unread_only?: boolean;
  limit?: number;
  offset?: number;
}

export interface ProtocolStats {
  total_value_locked: number;
  total_bets: number;
  total_pools: number;
}

export interface FeeInfo {
  treasury_fee_bps: number;
  referral_fee_bps: number;
}

export interface PricesResponse {
  prices: unknown;
  timestamp?: string | null;
}

export interface LeaderboardEntry {
  user_address: string;
  rank: number;
  total_volume?: number | null;
  prediction_count?: number | null;
  total_winnings?: number | null;
  winning_predictions?: number | null;
  total_predictions?: number | null;
  win_rate?: number | null;
}

export interface LeaderboardResponse {
  leaderboard: LeaderboardEntry[];
  rank_by: string;
  limit: number;
  offset: number;
  period?: string | null;
}

export interface Prediction {
  pool_id: number;
  pool_name: string;
  pool_result?: string | null;
  outcome: number;
  amount: number;
  created_at: string;
}

export interface PredictionHistory {
  address: string;
  predictions: Prediction[];
  limit: number;
  offset: number;
}

export interface UserPrediction {
  prediction_id: number;
  pool_id: number;
  pool_name: string;
  pool_category: string;
  pool_state: string;
  pool_total_stake: number;
  pool_result?: string | null;
  user_outcome: number;
  user_amount: number;
  is_winning_outcome?: boolean | null;
}

export interface UserPredictions {
  address: string;
  predictions: UserPrediction[];
  limit: number;
  offset: number;
  total_predictions: number;
}

export interface MarketPrediction {
  id: number;
  pool_id: number;
  user_address: string;
  outcome: number;
  amount: number;
  created_at: string;
}

export interface MarketPredictionsPage {
  market_id: number;
  predictions: MarketPrediction[];
  limit: number;
  next_cursor?: string | null;
  total?: number | null;
}

export interface UserProfile {
  total_predictions: number;
  wins: number;
  losses: number;
  pending: number;
  win_rate: number;
  total_staked: number;
  total_earnings: number;
  active_positions: number;
}

export interface UserInterests {
  address: string;
  interests: string[];
}

export interface ReferralEarning {
  pool_id: number;
  pool_name: string;
  total_earned: number;
  referral_count: number;
}

export interface ReferralEarnings {
  referrer: string;
  total_earned: number;
  pools: ReferralEarning[];
}

export interface CreatorStats {
  creator: string;
  pools_created: number;
  pools_reward_eligible: number;
  total_volume: number;
}

export interface ApiErrorBody {
  code?: string;
  message?: string;
  request_id?: string;
  requestId?: string;
  retryable?: boolean;
  details?: unknown;
}

export interface ApiSuccessEnvelope<T> {
  status: 'success';
  data: T;
}

export interface ApiErrorEnvelope {
  status: 'error';
  error: ApiErrorBody;
}

export type ApiEnvelope<T> = ApiSuccessEnvelope<T> | ApiErrorEnvelope;

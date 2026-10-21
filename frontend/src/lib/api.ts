import { API_BASE_URL } from './config';

export interface TokenAmount {
  /** Integer base units as a decimal string (no floating point). */
  baseUnits: string;
  /** Number of decimals for the token. */
  decimals: number;
}

/**
 * Centralized token precision policy. All token amounts crossing API or wallet
 * boundaries are represented as integer base units (decimal strings) to avoid
 * JavaScript floating-point rounding. Display formatting is derived from base
 * units only.
 */
export const TOKEN_DECIMALS = 18;

const DECIMAL_PATTERN = /^-?\d+(\.\d+)?$/;

/**
 * Parse a human-readable decimal string into integer base units.
 * Returns a decimal string of base units. Throws on malformed input.
 */
export function toBaseUnits(amount: string, decimals: number = TOKEN_DECIMALS): string {
  const trimmed = amount.trim();
  if (!DECIMAL_PATTERN.test(trimmed)) {
    throw new Error(`Invalid token amount: ${amount}`);
  }
  const negative = trimmed.startsWith('-');
  const unsigned = negative ? trimmed.slice(1) : trimmed;
  const [whole, fraction = ''] = unsigned.split('.');
  if (fraction.length > decimals) {
    throw new Error(`Too many decimal places for ${decimals}-decimal token: ${amount}`);
  }
  const paddedFraction = fraction.padEnd(decimals, '0');
  const combined = `${whole}${paddedFraction}`.replace(/^0+(?=\d)/, '');
  const normalized = combined === '' ? '0' : combined;
  return negative && normalized !== '0' ? `-${normalized}` : normalized;
}

/**
 * Format integer base units into a human-readable decimal string.
 * Trailing zeros in the fractional part are trimmed; whole values have no
 * fractional component.
 */
export function fromBaseUnits(baseUnits: string, decimals: number = TOKEN_DECIMALS): string {
  if (!/^-?\d+$/.test(baseUnits)) {
    throw new Error(`Invalid base units: ${baseUnits}`);
  }
  const negative = baseUnits.startsWith('-');
  const unsigned = negative ? baseUnits.slice(1) : baseUnits;
  const padded = unsigned.padStart(decimals + 1, '0');
  const whole = padded.slice(0, padded.length - decimals);
  const fraction = padded.slice(padded.length - decimals).replace(/0+$/, '');
  const sign = negative && (whole !== '0' || fraction !== '') ? '-' : '';
  return fraction.length > 0 ? `${sign}${whole}.${fraction}` : `${sign}${whole}`;
}

/**
 * Format base units for display with a fixed number of fractional digits.
 */
export function formatTokenAmount(
  baseUnits: string,
  decimals: number = TOKEN_DECIMALS,
  displayDecimals: number = 4,
): string {
  const value = fromBaseUnits(baseUnits, decimals);
  const negative = value.startsWith('-');
  const unsigned = negative ? value.slice(1) : value;
  const [whole, fraction = ''] = unsigned.split('.');
  const shownFraction = fraction.slice(0, displayDecimals).padEnd(displayDecimals, '0');
  const sign = negative ? '-' : '';
  return displayDecimals > 0 ? `${sign}${whole}.${shownFraction}` : `${sign}${whole}`;
}

/**
 * Add two base-unit amounts without floating point.
 */
export function addBaseUnits(a: string, b: string): string {
  return (BigInt(a) + BigInt(b)).toString();
}

/**
 * Subtract base-unit amounts without floating point.
 */
export function subBaseUnits(a: string, b: string): string {
  return (BigInt(a) - BigInt(b)).toString();
}

/**
 * Multiply base units by a rational rate (numerator/denominator) using integer
 * arithmetic, truncating toward zero. Useful for fee and payout estimates.
 */
export function mulBaseUnitsByRate(baseUnits: string, numerator: bigint, denominator: bigint): string {
  if (denominator === 0n) {
    throw new Error('Denominator must not be zero');
  }
  return ((BigInt(baseUnits) * numerator) / denominator).toString();
}

export interface StakePayload {
  amount: string;
}

export interface StakeResponse {
  amount: string;
}

export interface BalanceResponse {
  balance: string;
}

export interface PayoutEstimateResponse {
  estimate: string;
  fee: string;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
    ...init,
  });
  if (!response.ok) {
    throw new Error(`Request failed: ${response.status}`);
  }
  return (await response.json()) as T;
}

/**
 * Submit a stake. The human-readable amount is converted to integer base units
 * before being sent to the API.
 */
export async function submitStake(amount: string, decimals: number = TOKEN_DECIMALS): Promise<StakeResponse> {
  const payload: StakePayload = { amount: toBaseUnits(amount, decimals) };
  return request<StakeResponse>('/stake', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function fetchBalance(): Promise<BalanceResponse> {
  return request<BalanceResponse>('/balance');
}

export async function fetchPayoutEstimate(amount: string, decimals: number = TOKEN_DECIMALS): Promise<PayoutEstimateResponse> {
  const baseUnits = toBaseUnits(amount, decimals);
  return request<PayoutEstimateResponse>(`/payout-estimate?amount=${encodeURIComponent(baseUnits)}`);
}

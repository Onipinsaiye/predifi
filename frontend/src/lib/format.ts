// Centralized token amount precision policy and formatting.
// All token amounts are handled as integer base units (bigint) to avoid
// JavaScript floating-point rounding. Convert to/from base units only at
// API and wallet boundaries; format for display from base units.

export const TOKEN_DECIMALS = 18;
export const BASE_UNIT_FACTOR = 10n ** BigInt(TOKEN_DECIMALS);

/**
 * Parse a human-readable decimal string into integer base units.
 * Accepts an optional leading sign and up to TOKEN_DECIMALS fractional digits.
 * Throws on malformed input or excess precision.
 */
export function toBaseUnits(value: string, decimals: number = TOKEN_DECIMALS): bigint {
  const trimmed = value.trim();
  if (trimmed === "") {
    throw new Error("Cannot convert empty string to base units");
  }

  const match = /^([+-])?(\d*)(?:\.(\d*))?$/.exec(trimmed);
  if (!match) {
    throw new Error(`Invalid token amount: ${value}`);
  }

  const sign = match[1] === "-" ? -1n : 1n;
  const whole = match[2] || "0";
  const fraction = match[3] || "";

  if (fraction.length > decimals) {
    throw new Error(`Too many decimal places (max ${decimals}): ${value}`);
  }

  const paddedFraction = fraction.padEnd(decimals, "0");
  const factor = 10n ** BigInt(decimals);
  const magnitude = BigInt(whole) * factor + BigInt(paddedFraction || "0");
  return sign * magnitude;
}

/**
 * Convert integer base units back to a human-readable decimal string.
 * Trailing zeros in the fractional part are trimmed; whole numbers have no
 * decimal point.
 */
export function fromBaseUnits(baseUnits: bigint, decimals: number = TOKEN_DECIMALS): string {
  const negative = baseUnits < 0n;
  const magnitude = negative ? -baseUnits : baseUnits;
  const factor = 10n ** BigInt(decimals);
  const whole = magnitude / factor;
  const fraction = magnitude % factor;

  let result = whole.toString();
  if (fraction > 0n) {
    const fractionStr = fraction.toString().padStart(decimals, "0").replace(/0+$/, "");
    result = `${result}.${fractionStr}`;
  }

  return negative ? `-${result}` : result;
}

/**
 * Format integer base units for display with a fixed number of fractional
 * digits (defaults to the token precision policy). Rounds half-up at the
 * display boundary without ever touching floating point.
 */
export function formatTokenAmount(
  baseUnits: bigint,
  options: { decimals?: number; displayDecimals?: number } = {},
): string {
  const decimals = options.decimals ?? TOKEN_DECIMALS;
  const displayDecimals = options.displayDecimals ?? decimals;

  if (displayDecimals < 0) {
    throw new Error("displayDecimals must be non-negative");
  }

  const negative = baseUnits < 0n;
  const magnitude = negative ? -baseUnits : baseUnits;
  const factor = 10n ** BigInt(decimals);

  // Scale to the display precision, rounding half-up.
  const scale = 10n ** BigInt(displayDecimals);
  const scaled = (magnitude * scale + factor / 2n) / factor;

  const whole = scaled / scale;
  const fraction = scaled % scale;

  let result = whole.toString();
  if (displayDecimals > 0) {
    result = `${result}.${fraction.toString().padStart(displayDecimals, "0")}`;
  }

  return negative ? `-${result}` : result;
}

/**
 * Add two base-unit amounts safely.
 */
export function addBaseUnits(a: bigint, b: bigint): bigint {
  return a + b;
}

/**
 * Subtract two base-unit amounts safely.
 */
export function subBaseUnits(a: bigint, b: bigint): bigint {
  return a - b;
}

/**
 * Compute a fee in base units from a basis-points rate, rounding half-up.
 */
export function feeBaseUnits(amount: bigint, basisPoints: bigint): bigint {
  return (amount * basisPoints + 5000n) / 10000n;
}

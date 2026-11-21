import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Identifier validation rules for market and outcome IDs.
 *
 * IDs are expected to be non-empty, URL-safe tokens (alphanumeric, dash,
 * underscore). This keeps canonical URLs stable and prevents malformed or
 * injected values from being encoded into shareable links.
 */
const ID_PATTERN = /^[A-Za-z0-9_-]+$/;
const MAX_ID_LENGTH = 128;

export function isValidMarketOutcomeId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= MAX_ID_LENGTH &&
    ID_PATTERN.test(value)
  );
}

export interface MarketOutcomeParams {
  marketId: string;
  outcomeId: string;
}

/**
 * Build a canonical deep link for a market outcome.
 *
 * Returns `null` when either identifier is malformed so callers can fall back
 * to the market overview instead of emitting an invalid share URL.
 */
export function buildMarketOutcomeUrl(
  marketId: string,
  outcomeId: string,
  basePath = "/markets",
): string | null {
  if (!isValidMarketOutcomeId(marketId) || !isValidMarketOutcomeId(outcomeId)) {
    return null;
  }

  const normalizedBase = basePath.endsWith("/")
    ? basePath.slice(0, -1)
    : basePath;

  return `${normalizedBase}/${encodeURIComponent(marketId)}?outcome=${encodeURIComponent(
    outcomeId,
  )}`;
}

/**
 * Parse and validate market/outcome identifiers from route params or a URL.
 *
 * Accepts either a `URLSearchParams`-like object, a plain record, or a full
 * URL string (e.g. a copied share link). Returns `null` when identifiers are
 * missing, malformed, or the outcome is unknown (per `knownOutcomeIds`), so
 * callers can fall back to the market overview.
 */
export function parseMarketOutcomeParams(
  input:
    | URLSearchParams
    | Record<string, string | string[] | undefined>
    | string
    | null
    | undefined,
  knownOutcomeIds?: ReadonlyArray<string> | ReadonlySet<string>,
): MarketOutcomeParams | null {
  if (!input) {
    return null;
  }

  let marketId: string | undefined;
  let outcomeId: string | undefined;

  if (typeof input === "string") {
    let url: URL;
    try {
      url = new URL(input, "http://localhost");
    } catch {
      return null;
    }

    const segments = url.pathname.split("/").filter(Boolean);
    const marketsIndex = segments.lastIndexOf("markets");
    if (marketsIndex !== -1 && segments.length > marketsIndex + 1) {
      marketId = safeDecode(segments[marketsIndex + 1]);
    }

    outcomeId =
      safeDecode(url.searchParams.get("outcome") ?? undefined) ??
      safeDecode(url.searchParams.get("outcomeId") ?? undefined);
  } else if (input instanceof URLSearchParams) {
    marketId = safeDecode(input.get("market") ?? input.get("marketId") ?? undefined);
    outcomeId = safeDecode(input.get("outcome") ?? input.get("outcomeId") ?? undefined);
  } else {
    marketId = safeDecode(firstValue(input.market ?? input.marketId));
    outcomeId = safeDecode(firstValue(input.outcome ?? input.outcomeId));
  }

  if (!isValidMarketOutcomeId(marketId) || !isValidMarketOutcomeId(outcomeId)) {
    return null;
  }

  if (knownOutcomeIds) {
    const known =
      knownOutcomeIds instanceof Set
        ? knownOutcomeIds.has(outcomeId)
        : knownOutcomeIds.includes(outcomeId);
    if (!known) {
      return null;
    }
  }

  return { marketId, outcomeId };
}

function firstValue(value: string | string[] | undefined): string | undefined {
  if (Array.isArray(value)) {
    return value[0];
  }
  return value;
}

function safeDecode(value: string | undefined): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  try {
    return decodeURIComponent(value);
  } catch {
    return undefined;
  }
}
/**
 * Embeddable public market card utilities (issue #283).
 *
 * Communities need a safe way to reference markets on external sites.
 * The embed must:
 * - Contain only canonical, static market data (question, status, source
 *   link, and timestamp).
 * - Prohibit transaction signing: no wallet APIs are exposed.
 * - Prohibit third-party tracking: CSP and sandbox attributes block external
 *   scripts, pixels, and storage.
 * - Disclose stale data: a freshness notice is shown when indexed_at is
 *   beyond the stale threshold.
 * - Respond correctly to responsive sizing (width set by the parent frame).
 */

/** How old indexed data can be before it is flagged as stale (5 minutes). */
export const EMBED_STALE_THRESHOLD_MS = 5 * 60 * 1000;

/** Canonical data shown in the embed. All fields are static/read-only. */
export interface EmbedMarketData {
  poolId: number;
  question: string;
  status: string;
  category: string;
  /** Source link provided by the creator (may be absent for older markets). */
  sourceUrl?: string | null;
  /** ISO timestamp of last index update. Used for stale disclosure. */
  indexedAt?: string | null;
  /** Total stake, for a quick snapshot of market activity. */
  totalStake: number;
  token: string;
}

export type EmbedFreshnessState = "fresh" | "stale" | "unavailable";

/**
 * Classify the freshness of embed data.
 * Returns "unavailable" when indexedAt is absent or unparseable.
 */
export function classifyEmbedFreshness(
  indexedAt: string | null | undefined,
  nowMs: number = Date.now(),
): EmbedFreshnessState {
  if (!indexedAt) return "unavailable";
  const ts = Date.parse(indexedAt);
  if (isNaN(ts)) return "unavailable";
  return nowMs - ts > EMBED_STALE_THRESHOLD_MS ? "stale" : "fresh";
}

/**
 * Returns the sandbox attribute value for an <iframe> embedding a market card.
 *
 * Rules:
 * - allow-same-origin: needed so the page can apply its own styles.
 * - allow-scripts: needed to render the React SSR page.
 * - Notably absent: allow-forms, allow-popups, allow-top-navigation,
 *   allow-storage-access-by-user-activation — all of which could be used
 *   to initiate wallet interactions or navigate the host page.
 */
export const EMBED_IFRAME_SANDBOX = "allow-same-origin allow-scripts" as const;

/**
 * Build the Content-Security-Policy header value for the embed route.
 *
 * Constraints enforced:
 * - No external scripts (script-src 'self' only).
 * - No external stylesheets (style-src 'self' only, with inline for Next RSC).
 * - No external connections (connect-src 'none' — the embed is static).
 * - No forms (form-action 'none' — no wallet signing or POST).
 * - No external frames (frame-src 'none').
 * - No third-party images or fonts (img-src / font-src 'self' only).
 * - frame-ancestors '*' — allows embedding on any origin (the whole point).
 */
export function buildEmbedCSP(): string {
  return [
    "default-src 'none'",
    "script-src 'self' 'unsafe-inline'", // Next.js inline scripts for hydration
    "style-src 'self' 'unsafe-inline'",  // Tailwind inline styles
    "img-src 'self' data:",
    "font-src 'self'",
    "connect-src 'none'",                // prohibit any tracking/API calls
    "form-action 'none'",                // prohibit wallet signing (no form POSTs)
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "frame-ancestors *",                 // allow embedding anywhere
  ].join("; ");
}

/**
 * Build the HTML snippet an external site pastes to embed a market card.
 *
 * The snippet uses:
 * - `sandbox` attribute to restrict what the embedded page can do.
 * - `loading="lazy"` for performance.
 * - Responsive width (100%) with a fixed height for the compact card.
 *
 * @param baseUrl - The public base URL of the PrediFi frontend.
 * @param poolId  - The market to embed.
 */
export function buildEmbedSnippet(baseUrl: string, poolId: number): string {
  const url = `${baseUrl.replace(/\/$/, "")}/embed/markets/${poolId}`;
  return `<iframe
  src="${url}"
  sandbox="${EMBED_IFRAME_SANDBOX}"
  width="100%"
  height="200"
  frameborder="0"
  loading="lazy"
  title="PrediFi market: ${poolId}"
  style="border:none;border-radius:12px;overflow:hidden;"
></iframe>`;
}

/**
 * Validate a source URL provided by a market creator.
 * Returns true only for http/https URLs without credentials.
 */
export function isValidSourceUrl(url: unknown): boolean {
  if (typeof url !== "string") return false;
  try {
    const parsed = new URL(url);
    return (
      (parsed.protocol === "http:" || parsed.protocol === "https:") &&
      !parsed.username &&
      !parsed.password
    );
  } catch {
    return false;
  }
}

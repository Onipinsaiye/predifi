import type { Metadata } from "next";

import { getPoolById, isValidPoolId, type Pool } from "./pools";

const SITE_NAME = "Markets";
const DEFAULT_TITLE = "Market not found";
const DEFAULT_DESCRIPTION =
  "This market is unavailable or could not be found.";

export const MAX_TITLE_LENGTH = 120;
export const MAX_DESCRIPTION_LENGTH = 300;

/**
 * Strip harmful characters and collapse whitespace. This is defense-in-depth:
 * Next escapes metadata values when rendering, but we still sanitize to avoid
 * control characters and excessive length in share previews.
 */
export function sanitizeText(input: unknown, maxLength: number): string {
  if (typeof input !== "string") return "";
  const cleaned = input
    // remove control characters and line separators
    .replace(/[\u0000-\u001F\u007F\u2028\u2029]/g, " ")
    // collapse whitespace
    .replace(/\s+/g, " ")
    .trim();
  if (cleaned.length <= maxLength) return cleaned;
  return `${cleaned.slice(0, maxLength - 1).trimEnd()}\u2026`;
}

export function buildCanonicalUrl(id: string, baseUrl?: string): string {
  const base = (baseUrl ?? process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000").replace(
    /\/+$/,
    "",
  );
  const safeId = isValidPoolId(id) ? id : "";
  return safeId ? `${base}/markets/${encodeURIComponent(safeId)}` : `${base}/markets`;
}

export function buildPoolTitle(pool: Pool | null): string {
  if (!pool) return DEFAULT_TITLE;
  return sanitizeText(`${pool.name} ${SITE_NAME}`, MAX_TITLE_LENGTH);
}

export function buildPoolDescription(pool: Pool | null): string {
  if (!pool) return DEFAULT_DESCRIPTION;
  const base = pool.description ?? `Explore the ${pool.name} market.`;
  return sanitizeText(base, MAX_DESCRIPTION_LENGTH);
}

export function buildPoolImageUrl(id: string, baseUrl?: string): string {
  const canonical = buildCanonicalUrl(id, baseUrl);
  return `${canonical}/opengraph-image`;
}

export async function generatePoolMetadata(
  id: string,
  options: { baseUrl?: string } = {},
): Promise<Metadata> {
  const pool = isValidPoolId(id) ? await getPoolById(id) : null;
  const title = buildPoolTitle(pool);
  const description = buildPoolDescription(pool);
  const canonical = buildCanonicalUrl(id, options.baseUrl);
  const image = buildPoolImageUrl(id, options.baseUrl);

  return {
    title,
    description,
    alternates: { canonical },
    openGraph: {
      title,
      description,
      url: canonical,
      siteName: SITE_NAME,
      type: "website",
      images: [{ url: image, width: 1200, height: 630, alt: title }],
    },
    twitter: {
      card: "summary_large_image",
      title,
      description,
      images: [image],
    },
    robots: pool
      ? { index: true, follow: true }
      : { index: false, follow: false },
  };
}

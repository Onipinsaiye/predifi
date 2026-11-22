/**
 * Embeddable public market card page (issue #283).
 *
 * Route: /embed/markets/[id]
 *
 * This page is designed to be loaded inside an <iframe> on external sites.
 * It is intentionally stripped of all interactive features:
 * - No wallet connection UI.
 * - No transaction signing.
 * - No third-party scripts or tracking pixels.
 * - No navigation that escapes the frame.
 *
 * The CSP header (set in the route segment config and middleware) allows
 * frame-ancestors '*' so any site can embed, but prohibits connect-src and
 * form-action so the frame cannot initiate wallet flows or external requests.
 *
 * Stale data is disclosed explicitly when indexed_at is beyond the threshold.
 */

import type { Metadata } from "next";
import { headers } from "next/headers";
import { ExternalLink, AlertTriangle, Clock, HelpCircle } from "lucide-react";
import { fetchPoolById } from "@/lib/api/pools";
import { isValidPoolId } from "@/lib/pools";
import {
  classifyEmbedFreshness,
  isValidSourceUrl,
  type EmbedFreshnessState,
} from "@/lib/embedCard";

export const metadata: Metadata = {
  robots: { index: false },
};

type PageProps = {
  params: { [key: string]: string | string[] | undefined };
};

function resolveId({ params }: PageProps): string {
  const raw = params.id;
  const id = Array.isArray(raw) ? raw[0] : raw;
  return typeof id === "string" ? id : "";
}

// Suppress the default layout's wallet/nav chrome for embed pages.
export const dynamic = "force-dynamic";

function FreshnessBadge({ state }: { state: EmbedFreshnessState }) {
  if (state === "unavailable") {
    return (
      <span className="inline-flex items-center gap-1 text-[10px] text-zinc-500">
        <HelpCircle className="h-3 w-3" aria-hidden="true" />
        Freshness unknown
      </span>
    );
  }
  if (state === "stale") {
    return (
      <span
        role="status"
        className="inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-400"
        title="Data may be behind the latest ledger"
      >
        <AlertTriangle className="h-3 w-3" aria-hidden="true" />
        Stale data
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 text-[10px] text-zinc-500">
      <Clock className="h-3 w-3" aria-hidden="true" />
      Live data
    </span>
  );
}

export default async function EmbedMarketPage({ params }: PageProps) {
  const id = resolveId({ params });

  // Retrieve nonce from middleware for inline styles, if present.
  const nonce = (await headers()).get("x-nonce") ?? undefined;
  void nonce; // used by Next to nonce inline scripts — no manual injection needed

  if (!isValidPoolId(id)) {
    return (
      <EmbedShell>
        <p className="text-sm text-zinc-500">Invalid market ID.</p>
      </EmbedShell>
    );
  }

  const pool = await fetchPoolById(id).catch(() => null);

  if (!pool) {
    return (
      <EmbedShell>
        <p className="text-sm text-zinc-500">Market unavailable.</p>
      </EmbedShell>
    );
  }

  const freshnessState = classifyEmbedFreshness(pool.indexed_at);
  const sourceUrl =
    isValidSourceUrl((pool as Record<string, unknown>).source_url)
      ? (pool as Record<string, unknown>).source_url as string
      : null;

  return (
    <EmbedShell>
      {/* Category + status row */}
      <div className="flex flex-wrap items-center gap-2 mb-2">
        <span className="rounded bg-white/10 px-2 py-0.5 text-[10px] uppercase tracking-wide text-zinc-400">
          {pool.category}
        </span>
        <span className="rounded bg-[#37B7C3]/10 px-2 py-0.5 text-[10px] uppercase tracking-wide text-[#37B7C3]">
          {pool.state}
        </span>
        <FreshnessBadge state={freshnessState} />
      </div>

      {/* Market question */}
      <h1 className="text-sm font-semibold text-white leading-snug mb-3 line-clamp-3">
        {pool.name}
      </h1>

      {/* Stake snapshot */}
      <p className="text-[11px] text-zinc-500 mb-3">
        {pool.total_stake.toLocaleString()}&nbsp;{pool.token} staked
      </p>

      {/* Source link (opens in a new tab, rel=noopener,noreferrer) */}
      {sourceUrl && (
        <a
          href={sourceUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 text-[11px] text-[#37B7C3] hover:underline"
        >
          <ExternalLink className="h-3 w-3" aria-hidden="true" />
          Source
        </a>
      )}

      {/* Attribution + canonical link */}
      <div className="mt-auto pt-2 border-t border-white/5 flex items-center justify-between">
        <span className="text-[9px] text-zinc-700 uppercase tracking-wide">
          PrediFi
        </span>
        <a
          href={`/markets/${pool.pool_id}`}
          target="_blank"
          rel="noopener noreferrer"
          className="text-[9px] text-zinc-700 hover:text-zinc-400 transition-colors"
        >
          View full market ↗
        </a>
      </div>

      {/* Stale-data disclosure */}
      {freshnessState === "stale" && (
        <p className="mt-1 text-[9px] text-amber-500/70">
          Data may not reflect the latest on-chain state.
        </p>
      )}
    </EmbedShell>
  );
}

/** Minimal chrome-free shell for the embed page. */
function EmbedShell({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width,initial-scale=1" />
        <meta name="robots" content="noindex" />
        {/* Prevent the embed from accessing parent storage via document.referrer */}
        <meta name="referrer" content="no-referrer" />
        <style>{`
          *,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
          html,body{background:#0a0a0a;color:#fff;font-family:system-ui,sans-serif;height:100%}
          body{display:flex;flex-direction:column;min-height:100%}
        `}</style>
      </head>
      <body>
        <main className="p-4 flex flex-col h-full text-sm">
          {children}
        </main>
      </body>
    </html>
  );
}

"use client";

/**
 * ResolutionEvidenceCard — shareable resolution evidence card for a resolved market.
 *
 * Issue #284: Create shareable resolution evidence cards.
 *
 * Design decisions:
 * - Card data is built deterministically from canonical resolution fields so
 *   the same market always produces the same card at the same URL.
 * - Disputed/superseded/challenged/refunded/unavailable-evidence cards are
 *   clearly distinguished from clean resolutions via status banners and labels.
 * - Evidence references and transaction links are shown in full; unavailable
 *   evidence is marked explicitly (not silently omitted).
 * - A "Copy share link" button copies the deterministic share URL.
 * - The component never connects to any wallet or signs transactions.
 */

import React, { useState } from "react";
import {
  CheckCircle,
  AlertTriangle,
  XCircle,
  HelpCircle,
  ExternalLink,
  Copy,
  Check,
  RefreshCw,
  Shield,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  type ResolutionCard,
  type ResolutionOutcome,
  buildResolutionShareUrl,
} from "@/lib/resolutionEvidence";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatDate(isoString: string): string {
  try {
    return new Intl.DateTimeFormat("en-US", {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: "UTC",
    }).format(new Date(isoString));
  } catch {
    return isoString;
  }
}

function shortAddress(addr: string): string {
  if (addr.length <= 12) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

// ---------------------------------------------------------------------------
// Status banner per outcome
// ---------------------------------------------------------------------------

function OutcomeBanner({ outcome, isDisputed }: { outcome: ResolutionOutcome; isDisputed: boolean }) {
  if (outcome === "resolved" && !isDisputed) {
    return (
      <div className="flex items-center gap-2 rounded-lg bg-emerald-500/10 border border-emerald-500/20 px-3 py-2 text-xs text-emerald-400">
        <CheckCircle className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span className="font-medium">Resolved</span>
        <span className="text-emerald-500/70">— outcome confirmed</span>
      </div>
    );
  }
  if (outcome === "challenged" || (outcome === "resolved" && isDisputed)) {
    return (
      <div className="flex items-center gap-2 rounded-lg bg-amber-500/10 border border-amber-500/20 px-3 py-2 text-xs text-amber-400">
        <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span className="font-medium">Challenged</span>
        <span className="text-amber-500/70">— result under dispute</span>
      </div>
    );
  }
  if (outcome === "refunded") {
    return (
      <div className="flex items-center gap-2 rounded-lg bg-blue-500/10 border border-blue-500/20 px-3 py-2 text-xs text-blue-400">
        <RefreshCw className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span className="font-medium">Refunded</span>
        <span className="text-blue-500/70">— market cancelled, stakes returned</span>
      </div>
    );
  }
  if (outcome === "unavailable") {
    return (
      <div className="flex items-center gap-2 rounded-lg bg-zinc-800 border border-zinc-700 px-3 py-2 text-xs text-zinc-400">
        <HelpCircle className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span className="font-medium">Evidence unavailable</span>
        <span className="text-zinc-600">— cannot be verified</span>
      </div>
    );
  }
  return null;
}

// ---------------------------------------------------------------------------
// Superseded notice
// ---------------------------------------------------------------------------

function SupersededNotice() {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-xs text-red-400"
    >
      <XCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <div>
        <p className="font-semibold">Superseded resolution</p>
        <p className="text-red-400/70">
          This card reflects data that was overturned by a successful challenge.
          The winning outcome and resolver data shown below are no longer authoritative.
        </p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Evidence list
// ---------------------------------------------------------------------------

function EvidenceList({
  references,
}: {
  references: ResolutionCard["evidenceReferences"];
}) {
  if (references.length === 0) {
    return (
      <p className="text-xs text-zinc-500 italic">No evidence references provided.</p>
    );
  }
  return (
    <ul className="space-y-1.5">
      {references.map((ref, i) => (
        <li key={i} className="flex items-start gap-2 text-xs">
          {ref.unavailable ? (
            <span className="mt-0.5 inline-flex items-center gap-1 text-zinc-600">
              <HelpCircle className="h-3 w-3 shrink-0" aria-hidden="true" />
              <span className="line-through text-zinc-700">{ref.label}</span>
              <span className="text-zinc-600 not-italic">(unavailable)</span>
            </span>
          ) : ref.url ? (
            <a
              href={ref.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-[#37B7C3] hover:underline"
            >
              <ExternalLink className="h-3 w-3 shrink-0" aria-hidden="true" />
              {ref.label}
            </a>
          ) : (
            <span className="text-zinc-400">{ref.label}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------------------
// Transaction links
// ---------------------------------------------------------------------------

function TransactionLinks({
  links,
}: {
  links: ResolutionCard["transactionLinks"];
}) {
  if (links.length === 0) return null;
  return (
    <ul className="space-y-1">
      {links.map((link, i) => (
        <li key={i} className="text-xs">
          <a
            href={link.explorerUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-zinc-400 hover:text-[#37B7C3] transition-colors"
            title={link.txHash}
          >
            <ExternalLink className="h-3 w-3 shrink-0" aria-hidden="true" />
            {link.label}
            <span className="font-mono text-[10px] text-zinc-600">
              {link.txHash.slice(0, 8)}…
            </span>
          </a>
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------------------
// Challenge records
// ---------------------------------------------------------------------------

function ChallengeList({ challenges }: { challenges: ResolutionCard["challenges"] }) {
  if (challenges.length === 0) return null;
  return (
    <div className="space-y-2">
      {challenges.map((c, i) => (
        <div
          key={i}
          className="rounded bg-amber-500/5 border border-amber-500/15 px-3 py-2 text-xs space-y-0.5"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="font-medium text-amber-400">
              Challenge by {shortAddress(c.challengerAddress)}
            </span>
            {c.outcome && (
              <span
                className={cn(
                  "rounded-full px-2 py-0.5 text-[10px] uppercase tracking-wide",
                  c.outcome === "upheld"
                    ? "bg-red-500/15 text-red-400"
                    : c.outcome === "rejected"
                      ? "bg-zinc-800 text-zinc-500"
                      : "bg-amber-500/15 text-amber-400",
                )}
              >
                {c.outcome}
              </span>
            )}
          </div>
          <p className="text-zinc-400">{c.reason}</p>
          <p className="text-zinc-600 text-[10px]">{formatDate(c.challengedAt)}</p>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Copy share link
// ---------------------------------------------------------------------------

function ShareLinkButton({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback: select text
    }
  };

  return (
    <button
      type="button"
      onClick={() => void handleCopy()}
      className="inline-flex items-center gap-1.5 rounded-md border border-white/10 bg-white/5 px-3 py-1.5 text-xs text-zinc-300 hover:bg-white/10 hover:border-white/20 transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#37B7C3]"
      aria-label="Copy share link"
    >
      {copied ? (
        <>
          <Check className="h-3 w-3 text-emerald-400" aria-hidden="true" />
          <span className="text-emerald-400">Copied!</span>
        </>
      ) : (
        <>
          <Copy className="h-3 w-3" aria-hidden="true" />
          Copy share link
        </>
      )}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export interface ResolutionEvidenceCardProps {
  card: ResolutionCard;
  /**
   * Public base URL of the frontend, used to build the share link.
   * Falls back to window.location.origin if not provided.
   */
  baseUrl?: string;
  className?: string;
}

/**
 * ResolutionEvidenceCard renders all canonical resolution data for a market.
 *
 * Key behaviours:
 * - Disputed/challenged/superseded data is clearly distinguished.
 * - Evidence references mark unavailable items explicitly.
 * - Transaction links are shown for on-chain verification.
 * - A deterministic share URL is copyable.
 */
export function ResolutionEvidenceCard({
  card,
  baseUrl,
  className,
}: ResolutionEvidenceCardProps) {
  const shareUrl = buildResolutionShareUrl(
    baseUrl ?? (typeof window !== "undefined" ? window.location.origin : ""),
    card.cardId,
  );

  const isSuperseded =
    card.challenges.some((c) => c.outcome === "upheld") &&
    card.winningOutcome === null;

  return (
    <article
      aria-labelledby={`rc-heading-${card.cardId}`}
      className={cn(
        "rounded-2xl border bg-[#121212] p-5 space-y-4",
        card.isDisputed ? "border-amber-500/20" : "border-white/10",
        className,
      )}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1 min-w-0">
          <div className="flex items-center gap-1.5 text-[10px] text-zinc-600 uppercase tracking-wide">
            <Shield className="h-3 w-3" aria-hidden="true" />
            Resolution Evidence
          </div>
          <h2
            id={`rc-heading-${card.cardId}`}
            className="text-sm font-semibold text-white leading-snug"
          >
            {card.question}
          </h2>
        </div>
        <ShareLinkButton url={shareUrl} />
      </div>

      {/* Superseded notice */}
      {isSuperseded && <SupersededNotice />}

      {/* Outcome banner */}
      <OutcomeBanner outcome={card.outcome} isDisputed={card.isDisputed} />

      {/* Status summary */}
      <p className="text-xs text-zinc-400">{card.statusSummary}</p>

      {/* Winning outcome */}
      {card.winningOutcome !== null && (
        <div className="rounded-lg bg-white/5 border border-white/10 px-4 py-3 space-y-0.5">
          <p className="text-[10px] text-zinc-600 uppercase tracking-wide">
            Winning outcome
          </p>
          <p className="text-sm font-medium text-white">
            {card.winningOutcomeLabel ?? `Outcome ${card.winningOutcome}`}
          </p>
        </div>
      )}

      {/* Resolution metadata */}
      <dl className="grid grid-cols-1 gap-y-2 sm:grid-cols-2 text-xs">
        <div>
          <dt className="text-zinc-600">Resolved by</dt>
          <dd className="text-zinc-300 font-mono">
            {shortAddress(card.resolverAddress)}
          </dd>
        </div>
        <div>
          <dt className="text-zinc-600">Resolved at</dt>
          <dd className="text-zinc-300">{formatDate(card.resolvedAt)}</dd>
        </div>
        <div>
          <dt className="text-zinc-600">Card ID</dt>
          <dd className="text-zinc-600 font-mono text-[10px]">{card.cardId}</dd>
        </div>
      </dl>

      {/* Evidence references */}
      <section aria-label="Evidence references">
        <h3 className="mb-2 text-[10px] uppercase tracking-wide text-zinc-600">
          Evidence
        </h3>
        <EvidenceList references={card.evidenceReferences} />
      </section>

      {/* Transaction links */}
      {card.transactionLinks.length > 0 && (
        <section aria-label="Transaction links">
          <h3 className="mb-2 text-[10px] uppercase tracking-wide text-zinc-600">
            On-chain transactions
          </h3>
          <TransactionLinks links={card.transactionLinks} />
        </section>
      )}

      {/* Challenge records */}
      {card.challenges.length > 0 && (
        <section aria-label="Challenge history">
          <h3 className="mb-2 text-[10px] uppercase tracking-wide text-zinc-600">
            Challenge history
          </h3>
          <ChallengeList challenges={card.challenges} />
        </section>
      )}

      {/* Disputed data notice */}
      {card.isDisputed && (
        <p className="text-[10px] text-amber-500/70 italic">
          This card contains disputed or potentially superseded data. Verify
          on-chain transactions before citing this evidence.
        </p>
      )}
    </article>
  );
}

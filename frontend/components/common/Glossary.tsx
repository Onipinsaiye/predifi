"use client";

import { HelpCircle } from "lucide-react";
import { GlossaryEntry, GLOSSARY, getGlossaryEntry } from "@/lib/glossary";
import { Tooltip } from "@/components/ui/tooltip";

export interface GlossaryTermProps {
  /** Glossary term key, e.g. "close", "resolve", "claim". */
  key: string;
  /** Optional custom label to display instead of the term itself. */
  label?: string;
  /** Tooltip placement. Defaults to "top". */
  placement?: "top" | "right" | "bottom" | "left";
  /** Additional className for the term element. */
  className?: string;
}

/**
 * GlossaryTerm — renders a term with a tooltip definition from the central
 * glossary. Used at market and transaction decision points so users can
 * understand protocol terms without leaving the flow.
 *
 * Returns null when the term key is not found in the glossary, so callers
 * can render a fallback or skip gracefully.
 */
export function GlossaryTerm({
  key,
  label,
  placement = "top",
  className,
}: GlossaryTermProps) {
  const entry: GlossaryEntry | null = getGlossaryEntry(key);
  if (!entry) return null;

  const displayLabel = label ?? entry.term;

  return (
    <Tooltip
      content={
        <div className="text-left space-y-1">
          <p className="text-xs font-medium text-primary-foreground">
            {entry.term}
          </p>
          <p className="text-xs text-muted-foreground">{entry.definition}</p>
          <p className="text-[10px] text-muted-foreground/80">
            Defined in the PrediFi market glossary
          </p>
        </div>
      }
      side={placement}
      asDescription
    >
      <span
        className={cn(
          "inline-flex items-center gap-1 cursor-help underline underline-offset-4 hover:text-primary transition-colors",
          "text-xs text-primary",
          className,
        )}
        aria-describedby={`glossary-${key}`}
      >
        {displayLabel}
        <HelpCircle className="h-3 w-3 shrink-0 opacity-60" aria-hidden="true" />
      </span>
    </Tooltip>
  );
}

/**
 * GlossaryPanel — renders the full glossary list for a dedicated glossary page
 * or modal. Terms are grouped alphabetically with related-term cross-links.
 */
export interface GlossaryPanelProps {
  /** Optional subset of term keys to display. Defaults to all entries. */
  keys?: string[];
  /** Heading text. */
  heading?: string;
  /** Accessibility label. */
  ariaLabel?: string;
}

export function GlossaryPanel({
  keys,
  heading = "Market terminology glossary",
  ariaLabel = "Market terminology glossary",
}: GlossaryPanelProps) {
  const entries = keys
    ? keys.map((k) => getGlossaryEntry(k)).filter((e): e is GlossaryEntry => e !== null)
    : Object.values(GlossaryEntryMap);

  return (
    <section aria-label={ariaLabel}>
      <h2 className="text-sm font-semibold text-white mb-4">{heading}</h2>
      <div className="space-y-3">
        {entries.map((entry) => (
          <GlossaryDetailEntry key={entry.key} entry={entry} />
        ))}
      </div>
    </section>
  );
}

function GlossaryDetailEntry({ entry }: { entry: GlossaryEntry }) {
  const relatedLinks = (entry.related ?? [])
    .map((k) => {
      const related = getGlossaryEntry(k);
      return related ? (
        <span key={k} className="text-[10px] text-zinc-500">
          See also{" "}
          <GlossaryTerm key={related.key} label={related.term} />
        </span>
      ) : null;
    });

  return (
    <div className="rounded-lg border border-white/10 bg-[#121212] p-3">
      <div className="flex items-start justify-between gap-2 mb-1">
        <h3 className="text-sm font-medium text-white">{entry.term}</h3>
      </div>
      <p className="text-sm text-zinc-300 mb-2">{entry.definition}</p>
      {entry.explanation && (
        <p className="text-xs text-zinc-500 mb-2">{entry.explanation}</p>
      )}
      {relatedLinks.length > 0 && (
        <p className="text-xs text-zinc-500">{relatedLinks}</p>
      )}
    </div>
  );
}

// Re-export all entries for use in GlossaryPanel without circular import.
const GlossaryEntryMap: Record<string, GlossaryEntry> = {
  close: GLOSSARY.close,
  resolve: GLOSSARY.resolve,
  claim: GLOSSARY.claim,
  probability: GLOSSARY.probability,
  odds: GLOSSARY.odds,
  stake: GLOSSARY.stake,
  "resolution-source": GLOSSARY["resolution-source"],
  payout: GLOSSARY.payout,
  position: GLOSSARY.position,
};

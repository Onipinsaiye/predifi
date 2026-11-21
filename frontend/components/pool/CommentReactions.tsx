"use client";

/**
 * CommentReactions — moderated reaction bar for a discussion comment.
 *
 * Issue #282: Add moderated reactions for market discussion.
 *
 * Design decisions:
 * - Fixed reaction set (👍 🔥 🤔 🎯 ⚠️) prevents arbitrary signals.
 * - One active reaction per wallet per comment; toggling the same type
 *   deactivates it; selecting another replaces it.
 * - Counts are labelled "approx." to communicate eventual consistency.
 * - A disclaimer note states explicitly that reactions do not affect
 *   resolution or price — required by issue #282 acceptance criteria.
 * - Wallet changes: when currentUserAddress changes the active highlight
 *   recalculates automatically from the store.
 */

import React, { useCallback, useEffect, useState } from "react";
import { Info } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  REACTION_TYPES,
  REACTION_LABELS,
  type ReactionType,
  type ReactionCounts,
  getReactionCounts,
  getWalletReaction,
  toggleReaction,
} from "@/lib/reactions";

export interface CommentReactionsProps {
  /** The comment these reactions belong to. */
  commentId: string;
  /** Connected wallet address. Undefined means the user is not connected. */
  currentUserAddress: string | undefined;
  className?: string;
}

/**
 * CommentReactions renders the five-reaction bar beneath a comment.
 * Counts are eventually consistent; active state is per-wallet.
 */
export function CommentReactions({
  commentId,
  currentUserAddress,
  className,
}: CommentReactionsProps) {
  const [counts, setCounts] = useState<ReactionCounts>(() =>
    getReactionCounts(commentId),
  );
  const [activeReaction, setActiveReaction] = useState<ReactionType | undefined>(
    () =>
      currentUserAddress
        ? getWalletReaction(commentId, currentUserAddress)
        : undefined,
  );

  // Re-sync when wallet address changes (handles account switch).
  useEffect(() => {
    setActiveReaction(
      currentUserAddress
        ? getWalletReaction(commentId, currentUserAddress)
        : undefined,
    );
    setCounts(getReactionCounts(commentId));
  }, [commentId, currentUserAddress]);

  const handleReact = useCallback(
    (reaction: ReactionType) => {
      if (!currentUserAddress) return;
      const result = toggleReaction(commentId, currentUserAddress, reaction);
      setCounts(result.counts);
      setActiveReaction(
        result.outcome === "removed" ? undefined : result.reaction,
      );
    },
    [commentId, currentUserAddress],
  );

  const total = Object.values(counts).reduce((s, n) => s + n, 0);

  return (
    <div className={cn("space-y-1.5", className)}>
      {/* Reaction buttons */}
      <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Reactions">
        {REACTION_TYPES.map((reaction) => {
          const count = counts[reaction] ?? 0;
          const isActive = activeReaction === reaction;
          const label = REACTION_LABELS[reaction];
          return (
            <button
              key={reaction}
              type="button"
              onClick={() => handleReact(reaction)}
              disabled={!currentUserAddress}
              aria-pressed={isActive}
              aria-label={`${label}${count > 0 ? `, ${count}` : ""}${isActive ? ", active" : ""}`}
              title={
                currentUserAddress
                  ? `${label}${isActive ? " (click to remove)" : ""}`
                  : "Connect wallet to react"
              }
              className={cn(
                "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[12px] transition-all",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#37B7C3] focus-visible:ring-offset-1",
                !currentUserAddress && "cursor-not-allowed opacity-60",
                isActive
                  ? "border-[#37B7C3]/60 bg-[#37B7C3]/10 text-[#37B7C3]"
                  : "border-white/10 bg-white/5 text-zinc-400 hover:border-white/20 hover:bg-white/10",
              )}
            >
              <span aria-hidden="true">{reaction}</span>
              {count > 0 && (
                <span className="tabular-nums text-[11px]">{count}</span>
              )}
            </button>
          );
        })}
      </div>

      {/* Eventual-consistency and no-effect disclaimer */}
      {total > 0 && (
        <p className="flex items-center gap-1 text-[10px] text-zinc-600">
          <Info className="h-2.5 w-2.5 shrink-0" aria-hidden="true" />
          Counts are approximate · reactions do not affect resolution or price
        </p>
      )}
      {total === 0 && !currentUserAddress && (
        <p className="text-[10px] text-zinc-700 italic">
          Connect wallet to react
        </p>
      )}
    </div>
  );
}

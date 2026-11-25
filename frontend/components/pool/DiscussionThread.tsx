"use client";

/**
 * DiscussionThread — structured off-chain comment thread for a market page.
 *
 * Issue #281: Add structured discussion threads to market pages.
 *
 * Design decisions:
 * - Comments are off-chain and explicitly labelled as such so users cannot
 *   mistake discussion activity for resolution signals or chain events.
 * - Author identity is the connected wallet address; display is truncated for
 *   readability.
 * - Content is rendered as text (no innerHTML) to prevent XSS.
 * - Edit history is disclosed on demand so the conversation is auditable.
 * - Soft deletion replaces content with a tombstone and preserves the author
 *   slot so thread structure is maintained.
 * - Report is idempotent: a wallet can only report a comment once.
 */

import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  MessageSquare,
  Send,
  Pencil,
  Trash2,
  Flag,
  Clock,
  ChevronDown,
  ChevronUp,
  AlertTriangle,
  Info,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  type Comment,
  COMMENT_MAX_LENGTH,
  MAX_THREAD_DEPTH,
  addComment,
  editComment,
  deleteComment,
  reportComment,
  getComments,
  validateCommentContent,
  seedDemoComments,
  DiscussionConflictError,
} from "@/lib/discussion";

/** Short wallet display: first 6 + … + last 4 chars. */
function shortAddress(addr: string): string {
  if (addr.length <= 12) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

/** Human-relative time label for a comment. */
function relativeTime(isoString: string): string {
  const diffMs = Date.now() - new Date(isoString).getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return "just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return `${Math.floor(diffSec / 86400)}d ago`;
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface CommentItemProps {
  comment: Comment;
  currentUserAddress: string | undefined;
  onEdit: (commentId: string, newContent: string, revision: number) => void;
  onDelete: (commentId: string) => void;
  onReport: (commentId: string) => void;
  onReply: (parentId: string) => void;
  /** True when this comment is at the maximum allowed nesting depth. */
  atMaxDepth: boolean;
}

function CommentItem({
  comment,
  currentUserAddress,
  onEdit,
  onDelete,
  onReport,
  onReply,
  atMaxDepth,
}: CommentItemProps) {
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState(comment.content);
  const [editError, setEditError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const isAuthor =
    currentUserAddress !== undefined &&
    currentUserAddress === comment.authorAddress;

  const hasAlreadyReported =
    currentUserAddress !== undefined &&
    comment.reportedBy.includes(currentUserAddress);

  const handleEditSubmit = () => {
    const err = validateCommentContent(editValue);
    if (err) {
      setEditError(err);
      return;
    }
    setEditError(null);
    // Pass the revision we loaded so the backend can reject stale writes.
    onEdit(comment.id, editValue, comment.revision);
    setEditing(false);
  };

  const handleEditKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      handleEditSubmit();
    }
    if (e.key === "Escape") {
      setEditing(false);
      setEditValue(comment.content);
    }
  };

  useEffect(() => {
    if (editing) textareaRef.current?.focus();
  }, [editing]);

  if (comment.status === "deleted") {
    return (
      <li className="py-3 border-b border-white/5 last:border-0">
        <p className="text-sm italic text-zinc-600">
          [comment deleted]
          {comment.children.length > 0 && (
            <span className="ml-1 text-zinc-700">
              — replies preserved
            </span>
          )}
        </p>
        {comment.children.length > 0 && (
          <ul className="mt-2 ml-4 space-y-0 border-l border-white/5 pl-3">
            {comment.children.map((child) => (
              <CommentItem
                key={child.id}
                comment={child}
                currentUserAddress={currentUserAddress}
                onEdit={onEdit}
                onDelete={onDelete}
                onReport={onReport}
                onReply={onReply}
                atMaxDepth={comment.depth + 1 >= MAX_THREAD_DEPTH}
              />
            ))}
          </ul>
        )}
      </li>
    );
  }

  return (
    <li className="py-3 border-b border-white/5 last:border-0">
      <div className="flex items-start gap-3">
        {/* Avatar placeholder */}
        <div
          aria-hidden="true"
          className="mt-0.5 h-7 w-7 shrink-0 rounded-full bg-gradient-to-br from-[#37B7C3]/40 to-[#37B7C3]/10 flex items-center justify-center text-[10px] font-semibold text-[#37B7C3]"
        >
          {comment.authorAddress.slice(1, 3).toUpperCase()}
        </div>

        <div className="flex-1 min-w-0">
          {/* Header row */}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mb-1">
            <span className="text-xs font-medium text-zinc-300">
              {shortAddress(comment.authorAddress)}
            </span>
            <span className="text-[10px] text-zinc-600 flex items-center gap-1">
              <Clock className="h-2.5 w-2.5" />
              {relativeTime(comment.createdAt)}
            </span>
            {comment.editHistory.length > 0 && (
              <button
                type="button"
                onClick={() => setShowHistory((v) => !v)}
                className="text-[10px] text-zinc-600 hover:text-zinc-400 flex items-center gap-0.5 transition-colors"
                aria-expanded={showHistory}
                aria-label="Show edit history"
              >
                (edited)
                {showHistory ? (
                  <ChevronUp className="h-2.5 w-2.5" />
                ) : (
                  <ChevronDown className="h-2.5 w-2.5" />
                )}
              </button>
            )}
            {comment.status === "reported" && (
              <span className="text-[10px] text-amber-400 flex items-center gap-0.5">
                <AlertTriangle className="h-2.5 w-2.5" />
                reported
              </span>
            )}
          </div>

          {/* Edit history */}
          {showHistory && comment.editHistory.length > 0 && (
            <div className="mb-2 rounded bg-white/5 px-3 py-2 text-[11px] text-zinc-500 space-y-1">
              {comment.editHistory.map((h, i) => (
                <p key={i}>
                  <span className="text-zinc-600">{relativeTime(h.editedAt)}: </span>
                  {h.previousContent}
                </p>
              ))}
            </div>
          )}

          {/* Content / edit form */}
          {editing ? (
            <div className="space-y-1.5">
              <textarea
                ref={textareaRef}
                value={editValue}
                onChange={(e) => {
                  setEditValue(e.target.value);
                  setEditError(null);
                }}
                onKeyDown={handleEditKeyDown}
                maxLength={COMMENT_MAX_LENGTH}
                rows={2}
                className="w-full resize-none rounded bg-white/5 border border-white/10 px-3 py-2 text-sm text-white placeholder-zinc-600 focus:outline-none focus:ring-1 focus:ring-[#37B7C3]"
                aria-label="Edit comment"
              />
              {editError && (
                <p className="text-[11px] text-red-400">{editError}</p>
              )}
              <div className="flex gap-2">
                <Button
                  size="small"
                  variant="primary"
                  onClick={handleEditSubmit}
                >
                  Save
                </Button>
                <Button
                  size="small"
                  variant="tertiary"
                  onClick={() => {
                    setEditing(false);
                    setEditValue(comment.content);
                    setEditError(null);
                  }}
                >
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            <p className="text-sm text-zinc-200 whitespace-pre-wrap break-words">
              {comment.content}
            </p>
          )}

          {/* Action row */}
          {!editing && (
            <div className="mt-1.5 flex items-center gap-3">
              {isAuthor && (
                <>
                  <button
                    type="button"
                    onClick={() => setEditing(true)}
                    className="text-[11px] text-zinc-600 hover:text-zinc-300 flex items-center gap-1 transition-colors"
                    aria-label="Edit comment"
                  >
                    <Pencil className="h-3 w-3" />
                    Edit
                  </button>
                  <button
                    type="button"
                    onClick={() => onDelete(comment.id)}
                    className="text-[11px] text-zinc-600 hover:text-red-400 flex items-center gap-1 transition-colors"
                    aria-label="Delete comment"
                  >
                    <Trash2 className="h-3 w-3" />
                    Delete
                  </button>
                </>
              )}
              {!isAuthor && currentUserAddress && (
                <button
                  type="button"
                  onClick={() => !hasAlreadyReported && onReport(comment.id)}
                  disabled={hasAlreadyReported}
                  className={cn(
                    "text-[11px] flex items-center gap-1 transition-colors",
                    hasAlreadyReported
                      ? "text-zinc-700 cursor-not-allowed"
                      : "text-zinc-600 hover:text-amber-400 cursor-pointer",
                  )}
                  aria-label={
                    hasAlreadyReported
                      ? "Already reported"
                      : "Report comment"
                  }
                >
                  <Flag className="h-3 w-3" />
                  {hasAlreadyReported ? "Reported" : "Report"}
                </button>
              )}
              {currentUserAddress && (
                <button
                  type="button"
                  onClick={() => onReply(comment.id)}
                  disabled={atMaxDepth}
                  className={cn(
                    "text-[11px] flex items-center gap-1 transition-colors",
                    atMaxDepth
                      ? "text-zinc-700 cursor-not-allowed"
                      : "text-zinc-600 hover:text-[#37B7C3] cursor-pointer",
                  )}
                  aria-label={
                    atMaxDepth
                      ? "Maximum reply depth reached"
                      : "Reply to comment"
                  }
                  title={
                    atMaxDepth
                      ? `Maximum thread depth (${MAX_THREAD_DEPTH}) reached`
                      : undefined
                  }
                >
                  <MessageSquare className="h-3 w-3" />
                  Reply
                </button>
              )}
            </div>
          )}

          {/* Nested replies */}
          {comment.children.length > 0 && (
            <ul className="mt-2 ml-4 space-y-0 border-l border-white/5 pl-3">
              {comment.children.map((child) => (
                <CommentItem
                  key={child.id}
                  comment={child}
                  currentUserAddress={currentUserAddress}
                  onEdit={onEdit}
                  onDelete={onDelete}
                  onReport={onReport}
                  onReply={onReply}
                  atMaxDepth={comment.depth + 1 >= MAX_THREAD_DEPTH}
                />
              ))}
            </ul>
          )}
        </div>
      </div>
    </li>
  );
}

// ---------------------------------------------------------------------------
// Compose box
// ---------------------------------------------------------------------------

interface ComposeBoxProps {
  onSubmit: (content: string) => void;
  disabled?: boolean;
  disabledReason?: string;
  /** When set, the box is composing a reply to this comment. */
  replyingTo?: string | null;
  onCancelReply?: () => void;
}

function ComposeBox({
  onSubmit,
  disabled,
  disabledReason,
  replyingTo,
  onCancelReply,
}: ComposeBoxProps) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (replyingTo) textareaRef.current?.focus();
  }, [replyingTo]);

  const handleSubmit = () => {
    const err = validateCommentContent(value);
    if (err) {
      setError(err);
      return;
    }
    setError(null);
    onSubmit(value.trim());
    setValue("");
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      handleSubmit();
    }
  };

  const remaining = COMMENT_MAX_LENGTH - value.length;

  return (
    <div className="mt-4">
      {disabled ? (
        <p className="text-sm text-zinc-500 italic">
          {disabledReason ?? "Connect your wallet to join the discussion."}
        </p>
      ) : (
        <div className="space-y-2">
          {replyingTo && (
            <div className="flex items-center justify-between rounded bg-white/5 px-3 py-1.5 text-[11px] text-zinc-400">
              <span>Replying to a comment</span>
              <button
                type="button"
                onClick={onCancelReply}
                className="text-zinc-500 hover:text-zinc-300 transition-colors"
              >
                Cancel
              </button>
            </div>
          )}
          <textarea
            ref={textareaRef}
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
              setError(null);
            }}
            onKeyDown={handleKeyDown}
            maxLength={COMMENT_MAX_LENGTH}
            rows={3}
            placeholder="Share your analysis or ask a question…"
            className="w-full resize-none rounded-lg bg-white/5 border border-white/10 px-3 py-2 text-sm text-white placeholder-zinc-600 focus:outline-none focus:ring-1 focus:ring-[#37B7C3]"
            aria-label="New comment"
          />
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              {error && <p className="text-[11px] text-red-400">{error}</p>}
              <p className="text-[10px] text-zinc-600">
                Ctrl+Enter to submit · {remaining} chars remaining
              </p>
            </div>
            <Button
              size="small"
              variant="primary"
              onClick={handleSubmit}
              icon={<Send className="h-3 w-3" />}
              iconPosition="right"
              disabled={value.trim().length === 0}
            >
              Post
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export interface DiscussionThreadProps {
  /** The prediction market this thread belongs to. */
  poolId: number;
  /** Connected wallet address. Undefined means the user is not connected. */
  currentUserAddress: string | undefined;
  /**
   * When true, seeds demo comments and disables network activity.
   * Used on demo/static pages.
   */
  demo?: boolean;
  className?: string;
}

/**
 * DiscussionThread renders the full off-chain comment thread for a market.
 *
 * Visual separation from resolution rules and chain events is enforced by the
 * section header copy and the disclaimer note. Content is never passed to
 * dangerouslySetInnerHTML — React's text renderer handles escaping.
 */
export function DiscussionThread({
  poolId,
  currentUserAddress,
  demo = false,
  className,
}: DiscussionThreadProps) {
  const [comments, setComments] = useState<Comment[]>([]);
  const [feedbackMessage, setFeedbackMessage] = useState<string | null>(null);
  const [replyTo, setReplyTo] = useState<string | null>(null);

  // Load comments on mount or when poolId changes.
  useEffect(() => {
    if (demo) seedDemoComments(poolId);
    setComments(getComments(poolId));
  }, [poolId, demo]);

  const refresh = useCallback(() => {
    setComments(getComments(poolId));
  }, [poolId]);

  const showFeedback = (msg: string) => {
    setFeedbackMessage(msg);
    setTimeout(() => setFeedbackMessage(null), 3000);
  };

  const handlePost = useCallback(
    (content: string) => {
      if (!currentUserAddress) return;
      try {
        addComment({
          poolId,
          authorAddress: currentUserAddress,
          content,
          parentId: replyTo ?? undefined,
        });
        setReplyTo(null);
        refresh();
      } catch (err) {
        showFeedback(
          err instanceof Error ? err.message : "Could not post comment.",
        );
      }
    },
    [poolId, currentUserAddress, refresh, replyTo],
  );

  const handleEdit = useCallback(
    (commentId: string, newContent: string, revision: number) => {
      if (!currentUserAddress) return;
      try {
        editComment({
          commentId,
          editorAddress: currentUserAddress,
          newContent,
          revision,
        });
        refresh();
      } catch (err) {
        if (err instanceof DiscussionConflictError) {
          showFeedback(
            "This comment was edited elsewhere. Reloading the latest version.",
          );
          refresh();
          return;
        }
        showFeedback(err instanceof Error ? err.message : "Could not edit comment.");
      }
    },
    [currentUserAddress, refresh],
  );

  const handleDelete = useCallback(
    (commentId: string) => {
      if (!currentUserAddress) return;
      try {
        deleteComment(commentId, currentUserAddress);
        refresh();
        showFeedback("Comment deleted.");
      } catch (err) {
        showFeedback(err instanceof Error ? err.message : "Could not delete comment.");
      }
    },
    [currentUserAddress, refresh],
  );

  const handleReport = useCallback(
    (commentId: string) => {
      if (!currentUserAddress) return;
      try {
        reportComment({ commentId, reporterAddress: currentUserAddress });
        refresh();
        showFeedback("Comment reported for moderation.");
      } catch (err) {
        showFeedback(err instanceof Error ? err.message : "Could not report comment.");
      }
    },
    [currentUserAddress, refresh],
  );

  const countVisible = (list: Comment[]): number =>
    list.reduce(
      (acc, c) =>
        acc +
        (c.status !== "deleted" ? 1 : 0) +
        countVisible(c.children),
      0,
    );

  const visibleCount = countVisible(comments);

  return (
    <section
      aria-labelledby="discussion-heading"
      className={cn(
        "rounded-2xl border border-white/10 bg-[#121212] p-5 space-y-4",
        className,
      )}
    >
      {/* Header — visually separated from resolution rules */}
      <div className="flex items-center justify-between">
        <h2
          id="discussion-heading"
          className="flex items-center gap-2 text-sm font-semibold text-white"
        >
          <MessageSquare className="h-4 w-4 text-[#37B7C3]" aria-hidden="true" />
          Discussion
          {visibleCount > 0 && (
            <span className="text-[11px] text-zinc-500">({visibleCount})</span>
          )}
        </h2>
        {demo && (
          <span className="text-[10px] text-amber-300 uppercase tracking-wide">
            demo
          </span>
        )}
      </div>

      {/* Disclaimer — explicit separation from on-chain data */}
      <div className="flex items-start gap-2 rounded-lg bg-white/5 px-3 py-2 text-[11px] text-zinc-500">
        <Info className="mt-0.5 h-3.5 w-3.5 shrink-0 text-zinc-600" aria-hidden="true" />
        <p>
          Discussion is off-chain and does not affect market resolution, prices,
          or on-chain state.
        </p>
      </div>

      {/* Feedback toast */}
      {feedbackMessage && (
        <p
          role="status"
          aria-live="polite"
          className="text-[11px] text-[#37B7C3] border border-[#37B7C3]/20 rounded px-3 py-1.5 bg-[#37B7C3]/5"
        >
          {feedbackMessage}
        </p>
      )}

      {/* Comment list */}
      {comments.length === 0 ? (
        <p className="text-sm text-zinc-600">
          No comments yet. Be the first to share your analysis.
        </p>
      ) : (
        <ul className="space-y-0 divide-y divide-white/0" aria-label="Comments">
          {comments.map((comment) => (
            <CommentItem
              key={comment.id}
              comment={comment}
              currentUserAddress={currentUserAddress}
              onEdit={handleEdit}
              onDelete={handleDelete}
              onReport={handleReport}
              onReply={setReplyTo}
              atMaxDepth={comment.depth >= MAX_THREAD_DEPTH}
            />
          ))}
        </ul>
      )}

      {/* Compose box */}
      <ComposeBox
        onSubmit={handlePost}
        disabled={!currentUserAddress || demo}
        disabledReason={
          demo
            ? "Discussion is disabled in demo mode."
            : "Connect your wallet to join the discussion."
        }
        replyingTo={replyTo}
        onCancelReply={() => setReplyTo(null)}
      />
    </section>
  );
}

/**
 * Discussion thread data model for market pages (issue #281).
 *
 * Comments are off-chain content associated with a prediction market by
 * pool_id. They carry a stable author identity (wallet address), creation
 * time, an ordered edit history, and a report action.
 *
 * This module defines the in-memory types and a client-side storage layer
 * that simulates the off-chain store. A real deployment replaces the storage
 * layer with an authenticated API; the component API stays the same.
 *
 * Design constraints:
 * - Comments never alter authoritative contract data.
 * - Authored identity is the connected wallet address (read from context).
 * - Rendered content is escaped at the component boundary to prevent XSS.
 * - Edit history is append-only; deletions are soft (content replaced with a
 *   tombstone, original author identity preserved).
 */

export interface CommentEdit {
  /** ISO timestamp of this revision. */
  editedAt: string;
  /** Previous content, preserved for history. */
  previousContent: string;
}

export type CommentStatus = "visible" | "deleted" | "reported";

export interface Comment {
  id: string;
  /** pool_id this comment belongs to. */
  poolId: number;
  /** Wallet address of the author. */
  authorAddress: string;
  /** Current visible content. Empty for deleted comments. */
  content: string;
  /** ISO timestamp of first creation. */
  createdAt: string;
  status: CommentStatus;
  /** Append-only edit history. */
  editHistory: CommentEdit[];
  /** Wallet addresses that reported this comment. */
  reportedBy: string[];
}

/** Payload required to post a new comment. */
export interface NewCommentPayload {
  poolId: number;
  authorAddress: string;
  content: string;
}

/** Payload required to edit an existing comment. */
export interface EditCommentPayload {
  commentId: string;
  editorAddress: string;
  newContent: string;
}

/** Payload required to report a comment. */
export interface ReportCommentPayload {
  commentId: string;
  reporterAddress: string;
}

/** Maximum characters allowed in a comment. */
export const COMMENT_MAX_LENGTH = 500;

/** Minimum non-whitespace characters required. */
export const COMMENT_MIN_LENGTH = 1;

function generateId(): string {
  return `comment_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`;
}

/**
 * Validate comment content.
 * Returns an error string on failure, or null on success.
 */
export function validateCommentContent(content: string): string | null {
  const trimmed = content.trim();
  if (trimmed.length < COMMENT_MIN_LENGTH) return "Comment cannot be empty.";
  if (trimmed.length > COMMENT_MAX_LENGTH)
    return `Comment must be ${COMMENT_MAX_LENGTH} characters or fewer.`;
  return null;
}

/**
 * In-memory comment store (replaces the real API in this implementation).
 * Keyed by pool_id for fast lookup.
 */
const commentStore = new Map<number, Comment[]>();

/** Retrieve all visible comments for a pool, ordered oldest-first. */
export function getComments(poolId: number): Comment[] {
  return (commentStore.get(poolId) ?? []).slice();
}

/** Add a new comment to the thread. Returns the created comment. */
export function addComment(payload: NewCommentPayload): Comment {
  const error = validateCommentContent(payload.content);
  if (error) throw new Error(error);

  const comment: Comment = {
    id: generateId(),
    poolId: payload.poolId,
    authorAddress: payload.authorAddress,
    content: payload.content.trim(),
    createdAt: new Date().toISOString(),
    status: "visible",
    editHistory: [],
    reportedBy: [],
  };

  const list = commentStore.get(payload.poolId) ?? [];
  commentStore.set(payload.poolId, [...list, comment]);
  return comment;
}

/** Edit an existing comment. Only the original author may edit. */
export function editComment(payload: EditCommentPayload): Comment {
  const { commentId, editorAddress, newContent } = payload;
  const error = validateCommentContent(newContent);
  if (error) throw new Error(error);

  for (const [poolId, list] of commentStore.entries()) {
    const idx = list.findIndex((c) => c.id === commentId);
    if (idx === -1) continue;

    const existing = list[idx];
    if (existing.authorAddress !== editorAddress) {
      throw new Error("Only the original author may edit this comment.");
    }
    if (existing.status === "deleted") {
      throw new Error("Deleted comments cannot be edited.");
    }

    const updated: Comment = {
      ...existing,
      content: newContent.trim(),
      editHistory: [
        ...existing.editHistory,
        { editedAt: new Date().toISOString(), previousContent: existing.content },
      ],
    };

    const nextList = [...list];
    nextList[idx] = updated;
    commentStore.set(poolId, nextList);
    return updated;
  }

  throw new Error(`Comment ${commentId} not found.`);
}

/** Soft-delete a comment. Only the original author may delete. */
export function deleteComment(commentId: string, requesterAddress: string): void {
  for (const [poolId, list] of commentStore.entries()) {
    const idx = list.findIndex((c) => c.id === commentId);
    if (idx === -1) continue;

    const existing = list[idx];
    if (existing.authorAddress !== requesterAddress) {
      throw new Error("Only the original author may delete this comment.");
    }

    const updated: Comment = {
      ...existing,
      content: "",
      status: "deleted",
    };

    const nextList = [...list];
    nextList[idx] = updated;
    commentStore.set(poolId, nextList);
    return;
  }

  throw new Error(`Comment ${commentId} not found.`);
}

/** Report a comment for moderation. Idempotent per wallet. */
export function reportComment(payload: ReportCommentPayload): Comment {
  const { commentId, reporterAddress } = payload;

  for (const [poolId, list] of commentStore.entries()) {
    const idx = list.findIndex((c) => c.id === commentId);
    if (idx === -1) continue;

    const existing = list[idx];
    if (existing.reportedBy.includes(reporterAddress)) {
      // Idempotent — already reported by this wallet.
      return existing;
    }

    const updated: Comment = {
      ...existing,
      reportedBy: [...existing.reportedBy, reporterAddress],
      status: "reported",
    };

    const nextList = [...list];
    nextList[idx] = updated;
    commentStore.set(poolId, nextList);
    return updated;
  }

  throw new Error(`Comment ${commentId} not found.`);
}

/** Seed demo comments for a pool (used in tests and demo mode). */
export function seedDemoComments(poolId: number): void {
  if ((commentStore.get(poolId) ?? []).length > 0) return;

  const demo: Comment[] = [
    {
      id: "demo_1",
      poolId,
      authorAddress: "GDEMO1AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      content: "Great market! The resolution source looks solid.",
      createdAt: new Date(Date.now() - 1000 * 60 * 60 * 2).toISOString(),
      status: "visible",
      editHistory: [],
      reportedBy: [],
    },
    {
      id: "demo_2",
      poolId,
      authorAddress: "GDEMO2AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      content:
        "Worth noting the oracle typically posts within 48 hours of market close.",
      createdAt: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
      status: "visible",
      editHistory: [],
      reportedBy: [],
    },
  ];

  commentStore.set(poolId, demo);
}

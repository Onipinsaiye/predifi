/**
 * Discussion thread data model for market pages (issue #281).
 *
 * Comments are off-chain content associated with a prediction market by
 * pool_id. They carry a stable author identity (wallet address), creation
 * time, an ordered edit history, and a report action.
 *
 * This module defines the in-memory types and a client-side storage layer
 * that talks to the versioned discussion API (issue #299). The API persists
 * comments, revisions, and moderation state in relational tables that are
 * kept separate from the indexed contract event tables.
 *
 * Design constraints:
 * - Comments never alter authoritative contract data.
 * - Authored identity is the connected wallet address (read from context).
 * - Rendered content is escaped at the component boundary to prevent XSS.
 * - Edit history is append-only; deletions are soft (content replaced with a
 *   tombstone, original author identity preserved).
 * - Thread depth and body size are bounded, enforced both client-side here
 *   and server-side by the API.
 */

export interface CommentEdit {
  /** ISO timestamp of this revision. */
  editedAt: string;
  /** Previous content, preserved for history. */
  previousContent: string;
}

export type CommentStatus = "visible" | "deleted" | "reported";

/** Moderation state as tracked by the backend. */
export type ModerationState = "none" | "reported" | "hidden" | "removed";

export interface Comment {
  id: string;
  /** pool_id this comment belongs to. */
  poolId: number;
  /** Parent comment id, or null for a top-level comment. */
  parentId: string | null;
  /** Wallet address of the author. */
  authorAddress: string;
  /** Current visible content. Empty for deleted comments. */
  content: string;
  /** ISO timestamp of first creation. */
  createdAt: string;
  status: CommentStatus;
  /** Monotonic revision counter, incremented on each successful edit. */
  revision: number;
  /** Backend moderation state. */
  moderationState: ModerationState;
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
  /** Optional parent comment id for threaded replies. */
  parentId?: string | null;
}

/** Payload required to edit an existing comment. */
export interface EditCommentPayload {
  commentId: string;
  editorAddress: string;
  newContent: string;
  /** Revision the editor last observed; used for optimistic concurrency. */
  expectedRevision?: number;
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

/** Maximum nesting depth for replies (top-level comment is depth 0). */
export const COMMENT_MAX_DEPTH = 5;

/** Base path of the versioned discussion API. */
export const DISCUSSION_API_BASE = "/api/v1/discussions";

/** Error raised when the API rejects a request. */
export class DiscussionApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
    this.name = "DiscussionApiError";
  }
}

/** Wire shape returned by the API. */
interface CommentDto {
  id: string;
  pool_id: number;
  parent_id: string | null;
  author_address: string;
  content: string;
  created_at: string;
  status: CommentStatus;
  revision: number;
  moderation_state: ModerationState;
  edit_history: CommentEdit[];
  reported_by: string[];
}

function fromDto(dto: CommentDto): Comment {
  return {
    id: dto.id,
    poolId: dto.pool_id,
    parentId: dto.parent_id,
    authorAddress: dto.author_address,
    content: dto.content,
    createdAt: dto.created_at,
    status: dto.status,
    revision: dto.revision,
    moderationState: dto.moderation_state,
    editHistory: dto.edit_history ?? [],
    reportedBy: dto.reported_by ?? [],
  };
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${DISCUSSION_API_BASE}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
  });

  if (!response.ok) {
    let message = `Discussion request failed (${response.status}).`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body?.error) message = body.error;
    } catch {
      // Ignore non-JSON error bodies.
    }
    throw new DiscussionApiError(message, response.status);
  }

  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
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
 * Compute the depth of a comment within its thread.
 * Top-level comments are depth 0. Returns -1 if the parent chain is broken
 * (e.g. a parent was hard-removed) or exceeds the depth bound.
 */
export function computeCommentDepth(
  comment: Comment,
  byId: Map<string, Comment>,
): number {
  let depth = 0;
  let current: Comment | undefined = comment;
  const seen = new Set<string>();

  while (current?.parentId) {
    if (seen.has(current.id)) return -1; // cycle guard
    seen.add(current.id);
    const parent = byId.get(current.parentId);
    if (!parent) return -1;
    depth += 1;
    if (depth > COMMENT_MAX_DEPTH) return -1;
    current = parent;
  }

  return depth;
}

/** Retrieve all comments for a pool, ordered oldest-first. */
export async function getComments(poolId: number): Promise<Comment[]> {
  const dtos = await request<CommentDto[]>(
    `/pools/${encodeURIComponent(String(poolId))}/comments`,
    { method: "GET" },
  );
  return dtos.map(fromDto);
}

/** Add a new comment to the thread. Returns the created comment. */
export async function addComment(payload: NewCommentPayload): Promise<Comment> {
  const error = validateCommentContent(payload.content);
  if (error) throw new Error(error);

  const parentId = payload.parentId ?? null;
  if (parentId !== null) {
    // Enforce bounded depth client-side before hitting the API.
    const existing = await getComments(payload.poolId);
    const byId = new Map(existing.map((c) => [c.id, c]));
    const parent = byId.get(parentId);
    if (!parent) throw new Error("Parent comment not found.");
    const parentDepth = computeCommentDepth(parent, byId);
    if (parentDepth < 0 || parentDepth + 1 > COMMENT_MAX_DEPTH) {
      throw new Error(
        `Replies cannot exceed ${COMMENT_MAX_DEPTH} levels of nesting.`,
      );
    }
  }

  const dto = await request<CommentDto>(
    `/pools/${encodeURIComponent(String(payload.poolId))}/comments`,
    {
      method: "POST",
      body: JSON.stringify({
        author_address: payload.authorAddress,
        content: payload.content.trim(),
        parent_id: parentId,
      }),
    },
  );
  return fromDto(dto);
}

/** Edit an existing comment. Only the original author may edit. */
export async function editComment(payload: EditCommentPayload): Promise<Comment> {
  const { commentId, editorAddress, newContent, expectedRevision } = payload;
  const error = validateCommentContent(newContent);
  if (error) throw new Error(error);

  const dto = await request<CommentDto>(
    `/comments/${encodeURIComponent(commentId)}`,
    {
      method: "PATCH",
      body: JSON.stringify({
        editor_address: editorAddress,
        content: newContent.trim(),
        expected_revision: expectedRevision,
      }),
    },
  );
  return fromDto(dto);
}

/** Soft-delete a comment. Only the original author may delete. */
export async function deleteComment(
  commentId: string,
  requesterAddress: string,
): Promise<void> {
  await request<void>(`/comments/${encodeURIComponent(commentId)}`, {
    method: "DELETE",
    body: JSON.stringify({ requester_address: requesterAddress }),
  });
}

/** Report a comment for moderation. Idempotent per wallet. */
export async function reportComment(
  payload: ReportCommentPayload,
): Promise<Comment> {
  const { commentId, reporterAddress } = payload;
  const dto = await request<CommentDto>(
    `/comments/${encodeURIComponent(commentId)}/reports`,
    {
      method: "POST",
      body: JSON.stringify({ reporter_address: reporterAddress }),
    },
  );
  return fromDto(dto);
}

/**
 * Seed demo comments for a pool (used in tests and demo mode).
 * Demo seeding is a no-op against the real API; kept for API stability.
 */
export async function seedDemoComments(_poolId: number): Promise<void> {
  // No-op: demo data is not written to the versioned discussion store.
}

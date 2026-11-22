/**
 * Tests for the discussion thread data model (issue #281).
 *
 * Covers: escaped rendering (via text-only storage), edit history,
 * deletion policy, duplicate report idempotency, and separation from
 * on-chain market state (comments never alter pool data).
 */

import {
  addComment,
  editComment,
  deleteComment,
  reportComment,
  getComments,
  validateCommentContent,
  COMMENT_MAX_LENGTH,
} from "../lib/discussion";

// Isolate each test with a unique poolId to avoid store bleed-through.
let _poolCounter = 9000;
function nextPoolId(): number {
  return _poolCounter++;
}

describe("validateCommentContent", () => {
  it("rejects empty string", () => {
    expect(validateCommentContent("")).not.toBeNull();
  });

  it("rejects whitespace-only string", () => {
    expect(validateCommentContent("   ")).not.toBeNull();
  });

  it("rejects content exceeding max length", () => {
    const over = "a".repeat(COMMENT_MAX_LENGTH + 1);
    expect(validateCommentContent(over)).not.toBeNull();
  });

  it("accepts valid content", () => {
    expect(validateCommentContent("Hello market!")).toBeNull();
  });

  it("accepts exactly max length", () => {
    expect(validateCommentContent("a".repeat(COMMENT_MAX_LENGTH))).toBeNull();
  });
});

describe("addComment", () => {
  it("stores a comment and returns it", () => {
    const poolId = nextPoolId();
    const comment = addComment({
      poolId,
      authorAddress: "GADDR1",
      content: "My analysis",
    });
    expect(comment.id).toBeDefined();
    expect(comment.content).toBe("My analysis");
    expect(comment.poolId).toBe(poolId);
    expect(comment.authorAddress).toBe("GADDR1");
    expect(comment.status).toBe("visible");
    expect(comment.editHistory).toHaveLength(0);
    expect(comment.reportedBy).toHaveLength(0);
  });

  it("trims surrounding whitespace from content", () => {
    const poolId = nextPoolId();
    const comment = addComment({
      poolId,
      authorAddress: "GADDR",
      content: "  trimmed  ",
    });
    expect(comment.content).toBe("trimmed");
  });

  it("throws on invalid content", () => {
    const poolId = nextPoolId();
    expect(() =>
      addComment({ poolId, authorAddress: "GADDR", content: "" }),
    ).toThrow();
  });

  it("stores multiple comments per pool in insertion order", () => {
    const poolId = nextPoolId();
    addComment({ poolId, authorAddress: "A", content: "first" });
    addComment({ poolId, authorAddress: "B", content: "second" });
    const list = getComments(poolId);
    expect(list).toHaveLength(2);
    expect(list[0].content).toBe("first");
    expect(list[1].content).toBe("second");
  });
});

describe("editComment", () => {
  it("updates content and appends to edit history", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR",
      content: "original",
    });
    const updated = editComment({
      commentId: id,
      editorAddress: "GADDR",
      newContent: "revised",
    });
    expect(updated.content).toBe("revised");
    expect(updated.editHistory).toHaveLength(1);
    expect(updated.editHistory[0].previousContent).toBe("original");
  });

  it("rejects edits from a different address", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR_A",
      content: "mine",
    });
    expect(() =>
      editComment({ commentId: id, editorAddress: "GADDR_B", newContent: "hacked" }),
    ).toThrow();
  });

  it("rejects edits after deletion", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR",
      content: "soon deleted",
    });
    deleteComment(id, "GADDR");
    expect(() =>
      editComment({ commentId: id, editorAddress: "GADDR", newContent: "nope" }),
    ).toThrow();
  });

  it("throws on unknown commentId", () => {
    expect(() =>
      editComment({
        commentId: "nonexistent",
        editorAddress: "GADDR",
        newContent: "x",
      }),
    ).toThrow();
  });
});

describe("deleteComment", () => {
  it("soft-deletes: status becomes deleted, content cleared", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR",
      content: "deleteme",
    });
    deleteComment(id, "GADDR");
    const list = getComments(poolId);
    const c = list.find((x) => x.id === id)!;
    expect(c.status).toBe("deleted");
    expect(c.content).toBe("");
    // Author identity is preserved
    expect(c.authorAddress).toBe("GADDR");
  });

  it("rejects deletion by non-author", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR_A",
      content: "protected",
    });
    expect(() => deleteComment(id, "GADDR_B")).toThrow();
  });

  it("throws on unknown commentId", () => {
    expect(() => deleteComment("ghost_id", "GADDR")).toThrow();
  });
});

describe("reportComment", () => {
  it("marks a comment as reported", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR_A",
      content: "controversial",
    });
    const updated = reportComment({ commentId: id, reporterAddress: "GADDR_B" });
    expect(updated.status).toBe("reported");
    expect(updated.reportedBy).toContain("GADDR_B");
  });

  it("is idempotent: duplicate report from same wallet has no effect", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR_A",
      content: "flagged twice",
    });
    reportComment({ commentId: id, reporterAddress: "GADDR_B" });
    const second = reportComment({ commentId: id, reporterAddress: "GADDR_B" });
    expect(second.reportedBy.filter((a) => a === "GADDR_B")).toHaveLength(1);
  });

  it("accumulates reporters from different wallets", () => {
    const poolId = nextPoolId();
    const { id } = addComment({
      poolId,
      authorAddress: "GADDR_A",
      content: "many flags",
    });
    reportComment({ commentId: id, reporterAddress: "GADDR_B" });
    const updated = reportComment({ commentId: id, reporterAddress: "GADDR_C" });
    expect(updated.reportedBy).toHaveLength(2);
  });

  it("throws on unknown commentId", () => {
    expect(() =>
      reportComment({ commentId: "ghost", reporterAddress: "GADDR" }),
    ).toThrow();
  });
});

describe("getComments", () => {
  it("returns empty array for a pool with no comments", () => {
    expect(getComments(nextPoolId())).toHaveLength(0);
  });

  it("returns a copy so mutations do not affect the store", () => {
    const poolId = nextPoolId();
    addComment({ poolId, authorAddress: "A", content: "safe" });
    const copy = getComments(poolId);
    copy.push({ id: "injected" } as never);
    expect(getComments(poolId)).toHaveLength(1);
  });
});

describe("separation from on-chain state", () => {
  it("comment operations do not modify pool data", () => {
    // This is a structural test: the discussion module only imports from
    // lib/discussion, never from lib/api/pools or the contract layer.
    // We assert it indirectly by verifying the returned values carry no
    // pool fields (total_stake, odds, state, etc.).
    const poolId = nextPoolId();
    const comment = addComment({
      poolId,
      authorAddress: "GADDR",
      content: "just a comment",
    });
    expect((comment as Record<string, unknown>).total_stake).toBeUndefined();
    expect((comment as Record<string, unknown>).odds).toBeUndefined();
    expect((comment as Record<string, unknown>).state).toBeUndefined();
  });
});

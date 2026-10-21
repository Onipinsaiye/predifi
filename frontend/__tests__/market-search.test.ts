import {
  searchMarkets,
  searchMarketItems,
  searchMarketsPaginated,
  normalizeWhitespace,
  normalizeSearchText,
  tokenizeSearchQuery,
  evaluateMarketMatch,
  compareSearchResults,
  matchesFilters,
  type SearchableMarket,
} from "@/lib/search/marketSearch";
import {
  REPRESENTATIVE_MARKET_CORPUS,
  EXACT_MATCH_FIXTURES,
  PARTIAL_MATCH_FIXTURES,
  TIED_MATCH_FIXTURES,
} from "@/lib/search/__fixtures__/marketCorpus";

describe("Market Search Engine", () => {
  // ── 1. Normalization ──────────────────────────────────────────────────────
  describe("Normalization of Case and Whitespace", () => {
    it("collapses multiple spaces, tabs, and newlines into single spaces", () => {
      expect(normalizeWhitespace("   hello    world   ")).toBe("hello world");
      expect(normalizeWhitespace("crypto\t\nprediction \n market")).toBe(
        "crypto prediction market",
      );
      expect(normalizeWhitespace("  \u00A0 stellar \u2000 soroban  ")).toBe("stellar soroban");
    });

    it("normalizes case to lowercase and trims whitespace", () => {
      expect(normalizeSearchText("  BiTcoIn  ")).toBe("bitcoin");
      expect(normalizeSearchText("STELLAR SOROBAN")).toBe("stellar soroban");
      expect(normalizeSearchText(null)).toBe("");
      expect(normalizeSearchText(undefined)).toBe("");
    });

    it("tokenizes search query into distinct terms", () => {
      expect(tokenizeSearchQuery("   btc    usdc   active  ")).toEqual(["btc", "usdc", "active"]);
      expect(tokenizeSearchQuery("   ")).toEqual([]);
    });

    it("produces identical search results regardless of query casing", () => {
      const lower = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "bitcoin");
      const upper = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "BITCOIN");
      const mixed = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "BiTcOiN");

      expect(lower.length).toBeGreaterThan(0);
      expect(lower.map((r) => r.item.pool_id)).toEqual(upper.map((r) => r.item.pool_id));
      expect(lower.map((r) => r.score)).toEqual(upper.map((r) => r.score));
      expect(lower.map((r) => r.item.pool_id)).toEqual(mixed.map((r) => r.item.pool_id));
    });

    it("produces identical search results regardless of surrounding or irregular whitespace", () => {
      const normal = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "soroban stellar");
      const withExtraSpaces = searchMarkets(
        REPRESENTATIVE_MARKET_CORPUS,
        "   soroban     \t  stellar \n  ",
      );

      expect(normal.map((r) => r.item.pool_id)).toEqual(withExtraSpaces.map((r) => r.item.pool_id));
      expect(normal.map((r) => r.score)).toEqual(withExtraSpaces.map((r) => r.score));
    });
  });

  // ── 2. Exact Match Fixtures ───────────────────────────────────────────────
  describe("Exact Match Assertions", () => {
    it("ranks exact title match higher than partial/longer title matches", () => {
      const results = searchMarkets(EXACT_MATCH_FIXTURES, "Bitcoin");
      expect(results.length).toBeGreaterThanOrEqual(2);

      // Pool 201 has exact name "Bitcoin"
      expect(results[0].item.pool_id).toBe(201);
      expect(
        results[0].explanation.fieldMatches.some(
          (m) => m.field === "title" && m.matchType === "exact",
        ),
      ).toBe(true);
      expect(results[0].score).toBeGreaterThan(results[1].score);
    });

    it("matches exact tag across tags array with explainable tier", () => {
      const results = searchMarkets(EXACT_MATCH_FIXTURES, "spot-etf");
      expect(results.length).toBe(1);
      expect(results[0].item.pool_id).toBe(202);

      const tagMatch = results[0].explanation.fieldMatches.find((m) => m.field === "tags");
      expect(tagMatch).toBeDefined();
      expect(tagMatch?.matchType).toBe("exact");
      expect(tagMatch?.matchedText).toBe("spot-etf");
    });

    it("matches exact creator address", () => {
      const results = searchMarkets(
        EXACT_MATCH_FIXTURES,
        "GEXACTCREATOR111111111111111111111111111111111111111111",
      );
      expect(results.length).toBe(2);
      expect(results[0].explanation.matchedFields).toContain("creator");
      expect(results[1].explanation.matchedFields).toContain("creator");
    });

    it("matches exact market state and explains the match", () => {
      const results = searchMarkets(EXACT_MATCH_FIXTURES, "settled");
      expect(results.length).toBe(1);
      expect(results[0].item.pool_id).toBe(203);
      expect(results[0].explanation.matchedFields).toContain("state");
      expect(results[0].explanation.summary).toContain("state");
    });
  });

  // ── 3. Partial Match Fixtures ─────────────────────────────────────────────
  describe("Partial Match Assertions (Prefix, Word Boundary, Substring)", () => {
    it("ranks title prefix match higher than word boundary and substring matches", () => {
      const results = searchMarkets(PARTIAL_MATCH_FIXTURES, "Stellar");

      // Pool 301: Title starts with "Stellar..." (prefix, weight 1000 * 0.8 = 800)
      // Pool 302: Title contains word "Stellar" (word_exact, weight 1000 * 0.7 = 700)
      // Pool 303: Title contains "Interstellar" (substring, weight 1000 * 0.3 = 300)
      // Pool 304: Description mentions "stellar" (description, weight 500 * 0.7 = 350)
      expect(results[0].item.pool_id).toBe(301);
      expect(results[1].item.pool_id).toBe(302);
      expect(results[0].score).toBeGreaterThan(results[1].score);
    });

    it("matches terms appearing only in the market description", () => {
      const results = searchMarkets(PARTIAL_MATCH_FIXTURES, "remittance");
      expect(results.length).toBe(1);
      expect(results[0].item.pool_id).toBe(301);
      expect(results[0].explanation.matchedFields).toContain("description");
      expect(results[0].explanation.matchedFields).not.toContain("title");
    });

    it("matches terms appearing in creator prefix", () => {
      const results = searchMarkets(PARTIAL_MATCH_FIXTURES, "GASTROPHYSICIST");
      expect(results.length).toBe(1);
      expect(results[0].item.pool_id).toBe(303);
      expect(results[0].explanation.matchedFields).toContain("creator");
    });
  });

  // ── 4. Empty and Whitespace Queries ───────────────────────────────────────
  describe("Empty and Whitespace-only Query Matches", () => {
    it("returns all items in deterministic tie-break order for empty query string", () => {
      const results = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "");
      expect(results.length).toBe(REPRESENTATIVE_MARKET_CORPUS.length);
      expect(results.every((r) => r.score === 0)).toBe(true);
      expect(results[0].explanation.summary).toContain("Empty query");

      // Assert ranks are assigned 1..N
      expect(results.map((r) => r.rank)).toEqual(
        Array.from({ length: results.length }, (_, i) => i + 1),
      );
    });

    it("handles whitespace-only queries identically to empty queries", () => {
      const fromEmpty = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "");
      const fromSpaces = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "     ");
      const fromTabs = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "\t\n  \r\n");

      expect(fromSpaces.map((r) => r.item.pool_id)).toEqual(fromEmpty.map((r) => r.item.pool_id));
      expect(fromTabs.map((r) => r.item.pool_id)).toEqual(fromEmpty.map((r) => r.item.pool_id));
    });
  });

  // ── 5. Tied Matches and Deterministic Ordering ────────────────────────────
  describe("Tied Matches and Deterministic Ordering", () => {
    it("breaks ties with total_stake when match scores and tiers are equal", () => {
      // Pair 1: pool 401 (stake 5000) and pool 402 (stake 15000) both match "trading"
      const results = searchMarkets(TIED_MATCH_FIXTURES, "trading");
      const tiedResults = results.filter((r) => r.item.pool_id === 401 || r.item.pool_id === 402);

      expect(tiedResults.length).toBe(2);
      expect(tiedResults[0].score).toBe(tiedResults[1].score);
      // Higher stake (402) ranks first
      expect(tiedResults[0].item.pool_id).toBe(402);
      expect(tiedResults[1].item.pool_id).toBe(401);
    });

    it("breaks ties with created_at timestamp when score and stake are equal", () => {
      // Pair 2: pool 403 (created Jan 1) and pool 404 (created Feb 1) both match "delta"
      const results = searchMarkets(TIED_MATCH_FIXTURES, "delta");
      const tiedResults = results.filter((r) => r.item.pool_id === 403 || r.item.pool_id === 404);

      expect(tiedResults.length).toBe(2);
      expect(tiedResults[0].score).toBe(tiedResults[1].score);
      // Newer market (404) ranks first
      expect(tiedResults[0].item.pool_id).toBe(404);
      expect(tiedResults[1].item.pool_id).toBe(403);
    });

    it("breaks remaining ties with canonical pool_id ascending", () => {
      // Pair 3: pool 405 and pool 406 have identical scores, stakes, and creation dates
      const results = searchMarkets(TIED_MATCH_FIXTURES, "echo");
      const tiedResults = results.filter((r) => r.item.pool_id === 405 || r.item.pool_id === 406);

      expect(tiedResults.length).toBe(2);
      expect(tiedResults[0].score).toBe(tiedResults[1].score);
      // Canonical pool_id ascending: 405 before 406
      expect(tiedResults[0].item.pool_id).toBe(405);
      expect(tiedResults[1].item.pool_id).toBe(406);
    });

    it("guarantees search output ordering is invariant to input array permutation", () => {
      const original = [...REPRESENTATIVE_MARKET_CORPUS];
      const reversed = [...REPRESENTATIVE_MARKET_CORPUS].reverse();
      const shuffled = [...REPRESENTATIVE_MARKET_CORPUS].sort((a, b) =>
        (a.name ?? "").localeCompare(b.name ?? ""),
      );

      const query = "crypto";
      const resultsFromOrig = searchMarkets(original, query);
      const resultsFromRev = searchMarkets(reversed, query);
      const resultsFromShuf = searchMarkets(shuffled, query);

      expect(resultsFromOrig.map((r) => r.item.pool_id)).toEqual(
        resultsFromRev.map((r) => r.item.pool_id),
      );
      expect(resultsFromOrig.map((r) => r.item.pool_id)).toEqual(
        resultsFromShuf.map((r) => r.item.pool_id),
      );
    });
  });

  // ── 6. Explainability ─────────────────────────────────────────────────────
  describe("Explainability and Scoring Diagnostics", () => {
    it("provides complete explainability breakdown for matched fields", () => {
      const results = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "Bitcoin Coinbase");
      expect(results.length).toBeGreaterThan(0);
      const top = results[0];

      expect(top.explanation.totalScore).toBeGreaterThan(0);
      expect(top.explanation.matchedFields).toContain("title");
      expect(top.explanation.matchedFields).toContain("description");
      expect(top.explanation.matchedTokens).toContain("bitcoin");
      expect(top.explanation.matchedTokens).toContain("coinbase");
      expect(top.explanation.summary).toContain("Score");
      expect(top.explanation.tieBreakKey).toBe(String(top.item.pool_id));
    });

    it("awards full-phrase bonus when multi-word query appears contiguously", () => {
      const phraseResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "smart contracts");
      const top = phraseResults[0];
      expect(top.item.pool_id).toBe(103);
      expect(top.explanation.phraseBonus).toBeGreaterThan(0);
      expect(top.explanation.summary).toContain("[phrase bonus]");
    });

    it("awards all-terms matched bonus when all query tokens are present", () => {
      const results = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "ethereum dencun 1000");
      const top = results[0];
      expect(top.item.pool_id).toBe(102);
      expect(top.explanation.coverageBonus).toBeGreaterThan(0);
      expect(top.explanation.summary).toContain("[all terms matched]");
    });
  });

  // ── 7. Preserving Ranking When Filtering ──────────────────────────────────
  describe("Preserving Ranking When Filtering", () => {
    it("strictly preserves the relative ranking of remaining markets when category filter is applied", () => {
      const query = "2026";
      const allResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query);

      // Filter by category "Crypto"
      const cryptoResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query, {
        filters: { category: "Crypto" },
      });

      // Every item in cryptoResults must be in allResults, in the exact same relative order
      const cryptoIdsInAll = allResults
        .filter((r) => r.item.category === "Crypto")
        .map((r) => r.item.pool_id);

      const cryptoIdsFiltered = cryptoResults.map((r) => r.item.pool_id);

      expect(cryptoIdsFiltered).toEqual(cryptoIdsInAll);
    });

    it("strictly preserves relative ranking when status / state filter is applied", () => {
      const query = "winner";
      const allResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query);

      const activeResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query, {
        filters: { status: "settled" },
      });

      const settledIdsInAll = allResults
        .filter((r) => r.item.state === "settled")
        .map((r) => r.item.pool_id);

      expect(activeResults.map((r) => r.item.pool_id)).toEqual(settledIdsInAll);
    });

    it("preserves relative ranking with custom filter predicates", () => {
      const query = "crypto";
      const allResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query);

      const highStakeResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query, {
        filters: {
          filterFn: (m) => (m.total_stake ?? 0) > 100000,
        },
      });

      const expectedIds = allResults
        .filter((r) => (r.item.total_stake ?? 0) > 100000)
        .map((r) => r.item.pool_id);

      expect(highStakeResults.map((r) => r.item.pool_id)).toEqual(expectedIds);
    });
  });

  // ── 8. Preserving Ranking When Paginating ──────────────────────────────────
  describe("Preserving Ranking When Paginating", () => {
    it("slices non-overlapping pages that preserve the identical ranked order of full results", () => {
      const query = "crypto";
      const allResults = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, query);
      const pageSize = 2;

      const page0 = searchMarketsPaginated(REPRESENTATIVE_MARKET_CORPUS, query, {
        page: 0,
        pageSize,
      });
      const page1 = searchMarketsPaginated(REPRESENTATIVE_MARKET_CORPUS, query, {
        page: 1,
        pageSize,
      });

      expect(page0.totalMatches).toBe(allResults.length);
      expect(page0.items.length).toBe(Math.min(pageSize, allResults.length));

      // Concatenating pages 0 and 1 yields the first 4 items of allResults in identical order
      const combined = [...page0.items, ...page1.items];
      expect(combined.map((m) => m.pool_id)).toEqual(
        allResults.slice(0, combined.length).map((r) => r.item.pool_id),
      );

      // Verify no duplicates
      const ids = combined.map((m) => m.pool_id);
      const uniqueIds = new Set(ids);
      expect(uniqueIds.size).toBe(ids.length);
    });

    it("correctly handles page index beyond available items", () => {
      const paginated = searchMarketsPaginated(REPRESENTATIVE_MARKET_CORPUS, "crypto", {
        page: 99,
        pageSize: 10,
      });
      expect(paginated.items).toEqual([]);
      expect(paginated.results).toEqual([]);
      expect(paginated.totalMatches).toBeGreaterThan(0);
    });
  });

  // ── 9. Representative Market Corpus Validation ────────────────────────────
  describe("Validation Against Representative Market Corpus", () => {
    it("searches across title, description, tags, creator, and state seamlessly", () => {
      // 1. Title search
      const titleRes = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "Artificial Intelligence");
      expect(titleRes[0].item.pool_id).toBe(107);

      // 2. Description search
      const descRes = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "Copernicus");
      expect(descRes[0].item.pool_id).toBe(110);
      expect(descRes[0].explanation.matchedFields).toContain("description");

      // 3. Tags search
      const tagRes = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "interest-rates");
      expect(tagRes[0].item.pool_id).toBe(108);
      expect(tagRes[0].explanation.matchedFields).toContain("tags");

      // 4. Creator search
      const creatorRes = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "GCLIMATESCIENCE");
      expect(creatorRes[0].item.pool_id).toBe(110);
      expect(creatorRes[0].explanation.matchedFields).toContain("creator");

      // 5. Market State search
      const closedRes = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "closed");
      expect(closedRes.length).toBeGreaterThan(0);
      expect(closedRes.some((r) => r.item.state === "closed")).toBe(true);
    });

    it("searchMarketItems returns plain array of items with identical ordering", () => {
      const results = searchMarkets(REPRESENTATIVE_MARKET_CORPUS, "ethereum");
      const items = searchMarketItems(REPRESENTATIVE_MARKET_CORPUS, "ethereum");

      expect(items.map((i) => i.pool_id)).toEqual(results.map((r) => r.item.pool_id));
    });
  });
});

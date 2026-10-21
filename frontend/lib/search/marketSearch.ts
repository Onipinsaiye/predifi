/**
 * Deterministic and Explainable Market Search Engine
 *
 * Provides stable, explainable search and ranking across prediction markets:
 * - Search fields: Title (Name), Description, Tags, Creator, and Market State.
 * - Normalization: Case-folding and whitespace collapsing for both query and fields.
 * - Matching rules: Hierarchical weighted scoring (Exact > Prefix > Word > Substring).
 * - Multi-term handling: Phrase bonuses and full-term coverage rewards.
 * - Explainability: Every result includes matched fields, match types, field-level scores,
 *   and human-readable summaries.
 * - Deterministic tie-breaking: Guarantees 100% stable ordering invariant to incidental
 *   client array ordering or network response ordering.
 * - Filtering and pagination: Preserves strict relative ranking across pages and filters.
 */

// ---------------------------------------------------------------------------
// Types & Interfaces
// ---------------------------------------------------------------------------

/**
 * Universal shape for searchable prediction markets / pools.
 * Compatible with both `Pool` (from `lib/api/pools`) and general market objects.
 */
export interface SearchableMarket {
  /** Numeric or string identifier */
  pool_id?: number | string;
  id?: number | string;
  /** Title or name of the market */
  name?: string;
  title?: string;
  /** Market description / details */
  description?: string | null;
  /** Tags assigned to the market */
  tags?: string[] | null;
  /** Creator address or username */
  creator?: string | null;
  /** Lifecycle state: e.g. "active", "closed", "settled" */
  state?: string | null;
  status?: string | null;
  marketState?: string | null;
  /** Market category */
  category?: string | null;
  /** Total stake in base units */
  total_stake?: number | null;
  /** Creation timestamp or ISO string */
  created_at?: string | number | null;
  /** End / close time */
  end_time?: number | null;
  /** Token denomination */
  token?: string | null;
}

export type SearchField = "title" | "description" | "tags" | "creator" | "state" | "category";

export type MatchTier =
  | "exact"
  | "prefix"
  | "word_exact"
  | "word_prefix"
  | "substring"
  | "phrase_exact"
  | "phrase_substring";

export interface FieldMatchDetail {
  field: SearchField;
  term: string;
  matchType: MatchTier;
  score: number;
  matchedText: string;
  explanation: string;
}

export interface SearchExplanation {
  /** Total computed relevance score */
  totalScore: number;
  /** Base score summed from individual term matches */
  baseScore: number;
  /** Additional bonus if multi-word query matched as a contiguous phrase */
  phraseBonus: number;
  /** Bonus awarded when market matches all query terms */
  coverageBonus: number;
  /** Distinct fields that had at least one match */
  matchedFields: SearchField[];
  /** Query terms that matched */
  matchedTokens: string[];
  /** Detailed breakdown per matched field and term */
  fieldMatches: FieldMatchDetail[];
  /** Maximum score from any single field match (used for tie-breaking) */
  maxFieldScore: number;
  /** Human-readable explanation of why this market was matched and ranked */
  summary: string;
  /** Deterministic tie-break identifier */
  tieBreakKey: string;
}

export interface SearchResult<T extends SearchableMarket = SearchableMarket> {
  /** The original market item */
  item: T;
  /** 1-based rank in the deterministic result list */
  rank: number;
  /** Total score */
  score: number;
  /** Detailed match explanation */
  explanation: SearchExplanation;
}

export interface MarketSearchFilters<T extends SearchableMarket = SearchableMarket> {
  category?: string;
  status?: string | string[];
  state?: string | string[];
  creator?: string;
  tags?: string[];
  /** Custom filter predicate */
  filterFn?: (item: T) => boolean;
}

export interface MarketSearchOptions<T extends SearchableMarket = SearchableMarket> {
  /** Optional filter criteria */
  filters?: MarketSearchFilters<T>;
  /** Minimum score required to be included. Defaults to 1 (all positive matches). When query is empty, defaults to 0. */
  minScore?: number;
  /** 0-based page index for pagination */
  page?: number;
  /** Page size for pagination */
  pageSize?: number;
  /** Offset pagination */
  offset?: number;
  /** Limit pagination */
  limit?: number;
}

export interface PaginatedSearchResults<T extends SearchableMarket = SearchableMarket> {
  results: SearchResult<T>[];
  items: T[];
  totalMatches: number;
  page: number;
  pageSize: number;
  totalPages: number;
  query: string;
  normalizedQuery: string;
}

// ---------------------------------------------------------------------------
// Scoring Weights & Configuration
// ---------------------------------------------------------------------------

export const FIELD_WEIGHTS: Record<SearchField, number> = {
  title: 1000,
  tags: 750,
  description: 500,
  creator: 300,
  category: 250,
  state: 200,
};

export const MATCH_MULTIPLIERS: Record<MatchTier, number> = {
  phrase_exact: 1.5,
  exact: 1.0,
  prefix: 0.8,
  word_exact: 0.7,
  phrase_substring: 0.6,
  word_prefix: 0.5,
  substring: 0.3,
};

// ---------------------------------------------------------------------------
// Normalization Utilities
// ---------------------------------------------------------------------------

/**
 * Normalizes whitespace in any string:
 * - Collapses runs of whitespace (spaces, tabs, newlines, non-breaking spaces) into a single space.
 * - Trims leading and trailing whitespace.
 */
export function normalizeWhitespace(str: unknown): string {
  if (typeof str !== "string") return "";
  return str.replace(/[\s\u00A0\u2000-\u200B\uFEFF]+/g, " ").trim();
}

/**
 * Normalizes text for search matching:
 * - Lowercases the string.
 * - Normalizes and collapses whitespace.
 */
export function normalizeSearchText(str: unknown): string {
  return normalizeWhitespace(str).toLowerCase();
}

/**
 * Splits normalized text into distinct search tokens.
 */
export function tokenizeSearchQuery(query: string): string[] {
  const normalized = normalizeSearchText(query);
  if (!normalized) return [];
  return normalized.split(" ").filter((tok) => tok.length > 0);
}

// ---------------------------------------------------------------------------
// Field Extraction Helpers
// ---------------------------------------------------------------------------

export function getMarketTitle(market: SearchableMarket): string {
  return normalizeSearchText(market.title ?? market.name ?? "");
}

export function getMarketDescription(market: SearchableMarket): string {
  return normalizeSearchText(market.description ?? "");
}

export function getMarketTags(market: SearchableMarket): string[] {
  if (!Array.isArray(market.tags)) return [];
  return market.tags.map((t) => normalizeSearchText(t)).filter(Boolean);
}

export function getMarketCreator(market: SearchableMarket): string {
  return normalizeSearchText(market.creator ?? "");
}

export function getMarketState(market: SearchableMarket): string {
  return normalizeSearchText(market.state ?? market.status ?? market.marketState ?? "");
}

export function getMarketCategory(market: SearchableMarket): string {
  return normalizeSearchText(market.category ?? "");
}

export function getMarketId(market: SearchableMarket): string {
  const id = market.pool_id ?? market.id;
  return id !== undefined && id !== null ? String(id) : "";
}

export function getMarketTotalStake(market: SearchableMarket): number {
  return typeof market.total_stake === "number" && !isNaN(market.total_stake)
    ? market.total_stake
    : 0;
}

export function getMarketCreatedAt(market: SearchableMarket): number {
  if (!market.created_at) return 0;
  if (typeof market.created_at === "number") return market.created_at;
  const parsed = Date.parse(market.created_at);
  return isNaN(parsed) ? 0 : parsed;
}

// ---------------------------------------------------------------------------
// Matching Engine
// ---------------------------------------------------------------------------

/**
 * Checks a single target text against a single query term and returns the highest match tier.
 */
function scoreTermAgainstField(
  term: string,
  targetText: string,
  field: SearchField,
): { matchType: MatchTier; score: number } | null {
  if (!targetText || !term) return null;

  const baseWeight = FIELD_WEIGHTS[field];

  // Exact match
  if (targetText === term) {
    const matchType: MatchTier = "exact";
    return {
      matchType,
      score: Math.round(baseWeight * MATCH_MULTIPLIERS[matchType]),
    };
  }

  // Prefix match
  if (targetText.startsWith(term)) {
    const matchType: MatchTier = "prefix";
    return {
      matchType,
      score: Math.round(baseWeight * MATCH_MULTIPLIERS[matchType]),
    };
  }

  // Word-level matching
  const words = targetText.split(" ");
  let hasWordExact = false;
  let hasWordPrefix = false;

  for (const word of words) {
    if (word === term) {
      hasWordExact = true;
      break;
    }
    if (word.startsWith(term)) {
      hasWordPrefix = true;
    }
  }

  if (hasWordExact) {
    const matchType: MatchTier = "word_exact";
    return {
      matchType,
      score: Math.round(baseWeight * MATCH_MULTIPLIERS[matchType]),
    };
  }

  if (hasWordPrefix) {
    const matchType: MatchTier = "word_prefix";
    return {
      matchType,
      score: Math.round(baseWeight * MATCH_MULTIPLIERS[matchType]),
    };
  }

  // Substring match
  if (targetText.includes(term)) {
    const matchType: MatchTier = "substring";
    return {
      matchType,
      score: Math.round(baseWeight * MATCH_MULTIPLIERS[matchType]),
    };
  }

  return null;
}

/**
 * Evaluates a market against a search query, computing its score and explainability breakdown.
 */
export function evaluateMarketMatch<T extends SearchableMarket>(
  market: T,
  query: string,
): SearchExplanation {
  const normalizedQuery = normalizeSearchText(query);
  const tokens = tokenizeSearchQuery(query);
  const tieBreakKey = getMarketId(market);

  // Handle empty query
  if (tokens.length === 0) {
    return {
      totalScore: 0,
      baseScore: 0,
      phraseBonus: 0,
      coverageBonus: 0,
      matchedFields: [],
      matchedTokens: [],
      fieldMatches: [],
      maxFieldScore: 0,
      summary: "Empty query — default ordering applied.",
      tieBreakKey,
    };
  }

  const title = getMarketTitle(market);
  const description = getMarketDescription(market);
  const tags = getMarketTags(market);
  const creator = getMarketCreator(market);
  const category = getMarketCategory(market);
  const state = getMarketState(market);

  const fieldMatches: FieldMatchDetail[] = [];
  const matchedTokensSet = new Set<string>();
  const matchedFieldsSet = new Set<SearchField>();
  let maxFieldScore = 0;

  // 1. Term-by-term matching
  for (const term of tokens) {
    // Check title
    const titleMatch = scoreTermAgainstField(term, title, "title");
    if (titleMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("title");
      if (titleMatch.score > maxFieldScore) maxFieldScore = titleMatch.score;
      fieldMatches.push({
        field: "title",
        term,
        matchType: titleMatch.matchType,
        score: titleMatch.score,
        matchedText: title,
        explanation: `Title matched term "${term}" (${titleMatch.matchType}, +${titleMatch.score})`,
      });
    }

    // Check tags
    let bestTagMatch: { matchType: MatchTier; score: number; tag: string } | null = null;
    for (const tag of tags) {
      const match = scoreTermAgainstField(term, tag, "tags");
      if (match && (!bestTagMatch || match.score > bestTagMatch.score)) {
        bestTagMatch = { ...match, tag };
      }
    }
    if (bestTagMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("tags");
      if (bestTagMatch.score > maxFieldScore) maxFieldScore = bestTagMatch.score;
      fieldMatches.push({
        field: "tags",
        term,
        matchType: bestTagMatch.matchType,
        score: bestTagMatch.score,
        matchedText: bestTagMatch.tag,
        explanation: `Tag "#${bestTagMatch.tag}" matched term "${term}" (${bestTagMatch.matchType}, +${bestTagMatch.score})`,
      });
    }

    // Check description
    const descMatch = scoreTermAgainstField(term, description, "description");
    if (descMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("description");
      if (descMatch.score > maxFieldScore) maxFieldScore = descMatch.score;
      fieldMatches.push({
        field: "description",
        term,
        matchType: descMatch.matchType,
        score: descMatch.score,
        matchedText: description.slice(0, 80),
        explanation: `Description matched term "${term}" (${descMatch.matchType}, +${descMatch.score})`,
      });
    }

    // Check creator
    const creatorMatch = scoreTermAgainstField(term, creator, "creator");
    if (creatorMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("creator");
      if (creatorMatch.score > maxFieldScore) maxFieldScore = creatorMatch.score;
      fieldMatches.push({
        field: "creator",
        term,
        matchType: creatorMatch.matchType,
        score: creatorMatch.score,
        matchedText: creator,
        explanation: `Creator matched term "${term}" (${creatorMatch.matchType}, +${creatorMatch.score})`,
      });
    }

    // Check category
    const catMatch = scoreTermAgainstField(term, category, "category");
    if (catMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("category");
      if (catMatch.score > maxFieldScore) maxFieldScore = catMatch.score;
      fieldMatches.push({
        field: "category",
        term,
        matchType: catMatch.matchType,
        score: catMatch.score,
        matchedText: category,
        explanation: `Category "${category}" matched term "${term}" (${catMatch.matchType}, +${catMatch.score})`,
      });
    }

    // Check market state
    const stateMatch = scoreTermAgainstField(term, state, "state");
    if (stateMatch) {
      matchedTokensSet.add(term);
      matchedFieldsSet.add("state");
      if (stateMatch.score > maxFieldScore) maxFieldScore = stateMatch.score;
      fieldMatches.push({
        field: "state",
        term,
        matchType: stateMatch.matchType,
        score: stateMatch.score,
        matchedText: state,
        explanation: `Market state "${state}" matched term "${term}" (${stateMatch.matchType}, +${stateMatch.score})`,
      });
    }
  }

  // Base score is sum of all individual field/term match scores
  const baseScore = fieldMatches.reduce((acc, m) => acc + m.score, 0);

  // 2. Exact Title Match & Phrase bonus
  let phraseBonus = 0;
  if (title === normalizedQuery) {
    phraseBonus += 1000;
    if (phraseBonus > maxFieldScore) maxFieldScore = phraseBonus;
    fieldMatches.push({
      field: "title",
      term: normalizedQuery,
      matchType: "phrase_exact",
      score: 1000,
      matchedText: title,
      explanation: `Title exactly matched entire search query (+1000)`,
    });
  } else if (tokens.length > 1) {
    if (title.includes(normalizedQuery)) {
      phraseBonus += 250;
      fieldMatches.push({
        field: "title",
        term: normalizedQuery,
        matchType: "phrase_substring",
        score: 250,
        matchedText: title,
        explanation: `Title contained full search phrase (+250)`,
      });
    } else if (description.includes(normalizedQuery)) {
      phraseBonus += 150;
      fieldMatches.push({
        field: "description",
        term: normalizedQuery,
        matchType: "phrase_substring",
        score: 150,
        matchedText: description.slice(0, 80),
        explanation: `Description contained full search phrase (+150)`,
      });
    }
  }

  // 3. Multi-term coverage bonus: rewards items matching all search terms
  let coverageBonus = 0;
  if (tokens.length > 1 && matchedTokensSet.size === tokens.length) {
    coverageBonus = tokens.length * 200;
  }

  const totalScore = baseScore + phraseBonus + coverageBonus;
  const matchedFields = Array.from(matchedFieldsSet);
  const matchedTokens = Array.from(matchedTokensSet);

  // Build human-readable summary
  let summary = "";
  if (totalScore === 0) {
    summary = "No match.";
  } else {
    const fieldParts = matchedFields.map((f) => {
      const fieldMatchesForF = fieldMatches.filter((m) => m.field === f);
      const topMatchType = fieldMatchesForF[0]?.matchType ?? "partial";
      return `${f} (${topMatchType})`;
    });
    summary = `Score ${totalScore}: matched in ${fieldParts.join(", ")}`;
    if (phraseBonus > 0) summary += " [phrase bonus]";
    if (coverageBonus > 0) summary += " [all terms matched]";
  }

  return {
    totalScore,
    baseScore,
    phraseBonus,
    coverageBonus,
    matchedFields,
    matchedTokens,
    fieldMatches,
    maxFieldScore,
    summary,
    tieBreakKey,
  };
}

// ---------------------------------------------------------------------------
// Deterministic Tie-Breaker
// ---------------------------------------------------------------------------

/**
 * Deterministic search result comparator.
 *
 * Implements a strict total ordering:
 * 1. Score: Higher relevance score first.
 * 2. Field Match Quality: Highest single field score descending.
 * 3. Field Priority: Title > Tags > Description > Creator > State.
 * 4. Total Stake: Higher stake first (if available).
 * 5. Created Time: Newer market first (if available).
 * 6. Canonical Identifier: pool_id ascending (numerical) or id string ascending.
 *
 * Because Rule 6 compares unique IDs, no two distinct items can tie.
 * This guarantees the output order is completely invariant to incidental
 * initial ordering in the client array.
 */
export function compareSearchResults<T extends SearchableMarket>(
  a: SearchResult<T>,
  b: SearchResult<T>,
): number {
  // 1. Total relevance score (descending)
  if (b.score !== a.score) {
    return b.score - a.score;
  }

  // 2. Highest single field match score (descending)
  if (b.explanation.maxFieldScore !== a.explanation.maxFieldScore) {
    return b.explanation.maxFieldScore - a.explanation.maxFieldScore;
  }

  // 3. Field priority hierarchy (Title > Tags > Description > Creator > Category > State)
  const getFieldPriorityRank = (fields: SearchField[]): number => {
    let rank = 0;
    if (fields.includes("title")) rank += 32;
    if (fields.includes("tags")) rank += 16;
    if (fields.includes("description")) rank += 8;
    if (fields.includes("creator")) rank += 4;
    if (fields.includes("category")) rank += 2;
    if (fields.includes("state")) rank += 1;
    return rank;
  };
  const prioA = getFieldPriorityRank(a.explanation.matchedFields);
  const prioB = getFieldPriorityRank(b.explanation.matchedFields);
  if (prioB !== prioA) {
    return prioB - prioA;
  }

  // 4. Total stake (descending)
  const stakeA = getMarketTotalStake(a.item);
  const stakeB = getMarketTotalStake(b.item);
  if (stakeB !== stakeA) {
    return stakeB - stakeA;
  }

  // 5. Creation timestamp (descending: newer first)
  const timeA = getMarketCreatedAt(a.item);
  const timeB = getMarketCreatedAt(b.item);
  if (timeB !== timeA) {
    return timeB - timeA;
  }

  // 6. Canonical unique identifier (deterministic tie-break)
  const idA = a.item.pool_id ?? a.item.id ?? "";
  const idB = b.item.pool_id ?? b.item.id ?? "";

  if (typeof idA === "number" && typeof idB === "number") {
    return idA - idB;
  }

  return String(idA).localeCompare(String(idB));
}

// ---------------------------------------------------------------------------
// Filtering Helper
// ---------------------------------------------------------------------------

/**
 * Checks whether a market passes the provided filter criteria.
 * Filters operate independently of scoring, ensuring relative rank is preserved.
 */
export function matchesFilters<T extends SearchableMarket>(
  market: T,
  filters?: MarketSearchFilters<T>,
): boolean {
  if (!filters) return true;

  // Custom filter function
  if (filters.filterFn && !filters.filterFn(market)) {
    return false;
  }

  // Category filter
  if (filters.category && filters.category.toLowerCase() !== "all") {
    const marketCat = normalizeSearchText(market.category ?? "");
    const targetCat = normalizeSearchText(filters.category);
    if (marketCat !== targetCat) return false;
  }

  // Status / State filter
  const targetState = filters.state ?? filters.status;
  if (targetState) {
    const marketState = getMarketState(market);
    if (Array.isArray(targetState)) {
      const allowed = targetState.map((s) => normalizeSearchText(s));
      if (!allowed.includes(marketState)) return false;
    } else {
      if (marketState !== normalizeSearchText(targetState)) return false;
    }
  }

  // Creator filter
  if (filters.creator) {
    const marketCreator = getMarketCreator(market);
    if (marketCreator !== normalizeSearchText(filters.creator)) return false;
  }

  // Tags filter (requires all specified tags to be present)
  if (filters.tags && filters.tags.length > 0) {
    const marketTags = getMarketTags(market);
    for (const reqTag of filters.tags) {
      if (!marketTags.includes(normalizeSearchText(reqTag))) return false;
    }
  }

  return true;
}

// ---------------------------------------------------------------------------
// Main Search Functions
// ---------------------------------------------------------------------------

/**
 * Performs deterministic, explainable search and ranking across a market corpus.
 *
 * @param corpus Array of prediction markets / pools.
 * @param query User search string (supports multi-term, irregular whitespace, mixed case).
 * @param options Filters, score thresholds, and pagination options.
 * @returns Array of SearchResult items with rank, score, and explanation.
 */
export function searchMarkets<T extends SearchableMarket>(
  corpus: T[],
  query: string,
  options: MarketSearchOptions<T> = {},
): SearchResult<T>[] {
  const normalizedQuery = normalizeSearchText(query);
  const isEmpty = normalizedQuery.length === 0;
  const minScore = options.minScore !== undefined ? options.minScore : isEmpty ? 0 : 1;

  // 1. Evaluate every item in corpus
  const evaluated: SearchResult<T>[] = [];

  for (const item of corpus) {
    // Check filters first
    if (!matchesFilters(item, options.filters)) {
      continue;
    }

    const explanation = evaluateMarketMatch(item, query);

    // Filter by minScore (positive score only for non-empty queries by default)
    if (explanation.totalScore >= minScore) {
      evaluated.push({
        item,
        score: explanation.totalScore,
        explanation,
        rank: 0, // Assigned after deterministic sort
      });
    }
  }

  // 2. Deterministic sort using comprehensive tie-break rules
  evaluated.sort(compareSearchResults);

  // 3. Assign 1-based ranks
  for (let i = 0; i < evaluated.length; i++) {
    evaluated[i].rank = i + 1;
  }

  // 4. Pagination (if requested)
  let offset = options.offset;
  let limit = options.limit;

  if (options.page !== undefined && options.pageSize !== undefined) {
    offset = options.page * options.pageSize;
    limit = options.pageSize;
  }

  if (offset !== undefined || limit !== undefined) {
    const start = offset ?? 0;
    const end = limit !== undefined ? start + limit : undefined;
    return evaluated.slice(start, end);
  }

  return evaluated;
}

/**
 * Convenience wrapper returning only the ranked items (unwrapped from SearchResult).
 */
export function searchMarketItems<T extends SearchableMarket>(
  corpus: T[],
  query: string,
  options: MarketSearchOptions<T> = {},
): T[] {
  return searchMarkets(corpus, query, options).map((res) => res.item);
}

/**
 * Full paginated search returning results, metadata, and pagination totals.
 * Strictly preserves ranking across pages.
 */
export function searchMarketsPaginated<T extends SearchableMarket>(
  corpus: T[],
  query: string,
  options: {
    page: number;
    pageSize: number;
    filters?: MarketSearchFilters<T>;
    minScore?: number;
  },
): PaginatedSearchResults<T> {
  const normalizedQuery = normalizeSearchText(query);
  const { page, pageSize, filters, minScore } = options;

  // Get all ranked results without pagination first
  const allResults = searchMarkets(corpus, query, { filters, minScore });
  const totalMatches = allResults.length;
  const totalPages = Math.ceil(totalMatches / pageSize) || 0;

  // Slice deterministically
  const start = page * pageSize;
  const paginatedResults = allResults.slice(start, start + pageSize);

  return {
    results: paginatedResults,
    items: paginatedResults.map((r) => r.item),
    totalMatches,
    page,
    pageSize,
    totalPages,
    query,
    normalizedQuery,
  };
}

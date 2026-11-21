import { API_BASE_URL, ApiError, fetchWithTimeout } from "@/lib/api/pools";

export interface DuplicateMarketCandidate {
  proposal_id: string;
  question: string;
  event_window_start: string;
  event_window_end: string;
  source: string;
  outcomes: string[];
  score: number;
}

export interface DuplicateMarketCheckResult {
  proposal_id: string;
  candidates: DuplicateMarketCandidate[];
  requires_distinction: boolean;
  submitted: boolean;
}

export interface MarketDraftInput {
  proposal_id: string;
  question: string;
  event_window_start: string;
  event_window_end: string;
  source: string;
  outcomes: string[];
  distinction_note?: string;
}

function isCandidate(value: unknown): value is DuplicateMarketCandidate {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.proposal_id === "string" &&
    typeof candidate.question === "string" &&
    typeof candidate.event_window_start === "string" &&
    typeof candidate.event_window_end === "string" &&
    typeof candidate.source === "string" &&
    Array.isArray(candidate.outcomes) &&
    candidate.outcomes.every((outcome) => typeof outcome === "string") &&
    typeof candidate.score === "number"
  );
}

export async function checkMarketDraftDuplicates(
  draft: MarketDraftInput,
): Promise<DuplicateMarketCheckResult> {
  const response = await fetchWithTimeout(
    `${API_BASE_URL}/api/v1/market-drafts/duplicates`,
    {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(draft),
    },
  );
  const body: unknown = await response.json();
  if (!response.ok) {
    const message =
      typeof body === "object" &&
      body !== null &&
      "error" in body &&
      typeof body.error === "object" &&
      body.error !== null &&
      "message" in body.error
        ? String(body.error.message)
        : `Duplicate check failed (HTTP ${response.status})`;
    throw new ApiError(message, response.status);
  }

  if (
    typeof body !== "object" ||
    body === null ||
    !("data" in body) ||
    typeof body.data !== "object" ||
    body.data === null
  ) {
    throw new ApiError("Invalid duplicate-check response", 500);
  }

  const data = body.data as Partial<DuplicateMarketCheckResult>;
  if (
    typeof data.proposal_id !== "string" ||
    !Array.isArray(data.candidates) ||
    !data.candidates.every(isCandidate) ||
    typeof data.requires_distinction !== "boolean" ||
    typeof data.submitted !== "boolean"
  ) {
    throw new ApiError("Invalid duplicate-check response", 500);
  }
  return {
    proposal_id: data.proposal_id,
    candidates: data.candidates,
    requires_distinction: data.requires_distinction,
    submitted: data.submitted,
  };
}

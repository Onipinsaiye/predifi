import type { Metadata } from "next";
import { useEffect, useState } from "react";

export const metadata: Metadata = {
  title: "Community",
  description:
    "Join the PrediFi community — connect with predictors, validators, and crypto enthusiasts building the future of decentralized prediction markets on Stellar.",
  openGraph: {
    title: "Community | PrediFi — Join the Decentralized Prediction Network",
    description:
      "Join the PrediFi community — connect with predictors, validators, and crypto enthusiasts building the future of decentralized prediction markets on Stellar.",
    url: "https://predifi.app/community",
    siteName: "PrediFi",
    images: [{ url: "https://predifi.app/logo.jpeg", width: 1200, height: 630, alt: "PrediFi Community" }],
    locale: "en_US",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Community | PrediFi — Join the Decentralized Prediction Network",
    description:
      "Join the PrediFi community — connect with predictors, validators, and crypto enthusiasts building the future of decentralized prediction markets on Stellar.",
    images: ["https://predifi.app/logo.jpeg"],
  },
};

/**
 * Public participation statistics.
 *
 * Only thresholded aggregates are published. Cohorts below the privacy
 * threshold are suppressed by the API and reported via `suppressed`.
 * Raw wallet-level records are never exposed here.
 */
type ParticipationStats = {
  /** Total number of participants included in the sampled cohort. */
  participants: number;
  /** Total number of predictions placed by the sampled cohort. */
  predictions: number;
  /** Number of distinct markets with at least one prediction. */
  activeMarkets: number;
  /** Number of cohorts suppressed because they fell below the privacy threshold. */
  suppressedCohorts: number;
  /** Minimum cohort size required for a cohort to be published. */
  privacyThreshold: number;
  /** ISO timestamp of when the aggregate was sampled. */
  sampledAt: string;
};

type StatsState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; data: ParticipationStats };

function isParticipationStats(value: unknown): value is ParticipationStats {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.participants === "number" &&
    typeof v.predictions === "number" &&
    typeof v.activeMarkets === "number" &&
    typeof v.suppressedCohorts === "number" &&
    typeof v.privacyThreshold === "number" &&
    typeof v.sampledAt === "string"
  );
}

function formatNumber(n: number): string {
  return new Intl.NumberFormat("en-US").format(n);
}

function formatSampledAt(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "UTC",
  });
}

function ParticipationStatistics() {
  const [state, setState] = useState<StatsState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();

    async function load() {
      try {
        const res = await fetch("/api/community/participation-stats", {
          signal: controller.signal,
          headers: { Accept: "application/json" },
        });
        if (!res.ok) {
          throw new Error(`Request failed with status ${res.status}`);
        }
        const json: unknown = await res.json();
        if (!isParticipationStats(json)) {
          throw new Error("Unexpected response shape");
        }
        if (!cancelled) {
          setState({ status: "ready", data: json });
        }
      } catch (err) {
        if (cancelled || controller.signal.aborted) return;
        setState({
          status: "error",
          message: err instanceof Error ? err.message : "Failed to load statistics",
        });
      }
    }

    load();
    return () => {
      cancelled = true;
      controller.abort();
    };
  }, []);

  return (
    <section aria-labelledby="participation-stats-heading" className="participation-stats">
      <h2 id="participation-stats-heading">Public Participation Statistics</h2>
      <p>
        These aggregates are published only when the cohort meets the privacy
        threshold. Individual wallet balances and raw wallet-level records are
        never shown.
      </p>

      {state.status === "loading" && <p role="status">Loading statistics…</p>}

      {state.status === "error" && (
        <p role="alert">Statistics are currently unavailable.</p>
      )}

      {state.status === "ready" && (
        <>
          <dl>
            <div>
              <dt>Participants</dt>
              <dd>{formatNumber(state.data.participants)}</dd>
            </div>
            <div>
              <dt>Predictions placed</dt>
              <dd>{formatNumber(state.data.predictions)}</dd>
            </div>
            <div>
              <dt>Active markets</dt>
              <dd>{formatNumber(state.data.activeMarkets)}</dd>
            </div>
            <div>
              <dt>Suppressed cohorts</dt>
              <dd>{formatNumber(state.data.suppressedCohorts)}</dd>
            </div>
          </dl>
          <p className="participation-stats__meta">
            Denominator: {formatNumber(state.data.participants)} participants in
            the sampled cohort. Cohorts smaller than{" "}
            {formatNumber(state.data.privacyThreshold)} participants are
            suppressed. Sampled at {formatSampledAt(state.data.sampledAt)} UTC.
          </p>
        </>
      )}
    </section>
  );
}

export default function Community(){
    return(
        <>
            <h1>Community Page</h1>
            <ParticipationStatistics />
        </>
    )
}
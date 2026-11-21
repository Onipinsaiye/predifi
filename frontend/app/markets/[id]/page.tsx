import type { Metadata } from "next";

import { generatePoolMetadata } from "../../../lib/metadata";
import { getPoolById, isValidPoolId } from "../../../lib/pools";

type PageProps = {
  params: { [key: string]: string | string[] | undefined };
  searchParams?: { [key: string]: string | string[] | undefined };
};

function resolveId({ params }: PageProps): string {
  const raw = params.id;
  const id = Array.isArray(raw) ? raw[0] : raw;
  return typeof id === "string" ? id : "";
}

function resolveOutcome({ searchParams }: PageProps): string | null {
  const raw = searchParams?.outcome;
  const outcome = Array.isArray(raw) ? raw[0] : raw;
  return typeof outcome === "string" && outcome.length > 0 ? outcome : null;
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const id = resolveId({ params });
  return generatePoolMetadata(id);
}

export default async function MarketPage({ params, searchParams }: PageProps) {
  const id = resolveId({ params });
  const pool = isValidPoolId(id) ? await getPoolById(id) : null;

  if (!pool) {
    return (
      <main className="mx-auto max-w-2xl px-6 py-16">
        <h1 className="text-2xl font-semibold">Market not found</h1>
        <p className="mt-4 text-gray-600">
          The market you are looking for is unavailable or could not be found.
        </p>
      </main>
    );
  }

  const requestedOutcome = resolveOutcome({ params, searchParams });
  const outcomes = Array.isArray(pool.outcomes) ? pool.outcomes : [];
  const selectedOutcome =
    requestedOutcome !== null
      ? outcomes.find((outcome) => outcome.id === requestedOutcome) ?? null
      : null;
  const outcomeFallback = requestedOutcome !== null && selectedOutcome === null;

  return (
    <main className="mx-auto max-w-2xl px-6 py-16">
      <h1 className="text-3xl font-semibold">{pool.name}</h1>
      {outcomeFallback ? (
        <p className="mt-4 text-sm text-amber-700" role="status">
          The selected outcome is no longer available. Showing the market overview.
        </p>
      ) : null}
      {selectedOutcome ? (
        <p className="mt-4 text-gray-700">
          Selected outcome: <span className="font-medium">{selectedOutcome.name}</span>
        </p>
      ) : null}
      {pool.description ? (
        <p className="mt-4 text-gray-600">{pool.description}</p>
      ) : null}
      <dl className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-3">
        <div className="rounded-border p-4">
          <dt className="text-sm text-gray-500">TVL</dt>
          <dd className="text-lg font-medium">${pool.tvL.toLocaleString()}</dd>
        </div>
        {typeof pool.apr === "number" ? (
          <div className="rounded-border p-4">
            <dt className="text-sm text-gray-500">APR</dt>
            <dd className="text-lg font-medium">{(pool.apr * 100).toFixed(2)}%</dd>
          </div>
        ) : null}
        {typeof pool.volume24h === "number" ? (
          <div className="rounded-border p-4">
            <dt className="text-sm text-gray-500">Volume (24h)</dt>
            <dd className="text-lg font-medium">${pool.volume24h.toLocaleString()}</dd>
          </div>
        ) : null}
      </dl>
    </main>
  );
}

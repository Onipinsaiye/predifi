import type { Metadata } from "next";
import { fetchPoolById } from "@/lib/api/pools";
import { PoolDetailClient } from "./PoolDetailClient";

const SITE_URL = "https://predifi.app";

function formatStake(amount: number, token: string): string {
  return `${amount.toLocaleString()} ${token}`;
}

function outcomeLabel(outcome: number, labels?: string[]): string {
  return labels?.[outcome] ?? `Outcome ${outcome + 1}`;
}

interface PageProps {
  params: Promise<{ id: string }>;
}

export async function generateMetadata({
  params,
}: PageProps): Promise<Metadata> {
  const { id } = await params;
  const pool = await fetchPoolById(id).catch(() => null);

  if (!pool) {
    return { title: "Pool Not Found | PrediFi" };
  }

  const title = `${pool.name} | PrediFi`;
  const topOutcome = [...pool.odds].sort((a, b) => b.stake - a.stake)[0];
  const description = topOutcome
    ? `${pool.category} prediction pool on PrediFi — ${formatStake(pool.total_stake, pool.token)} staked. Leading: ${outcomeLabel(topOutcome.outcome, pool.outcome_descriptions)} at ${topOutcome.odds.toFixed(2)}x odds.`
    : `${pool.category} prediction pool on PrediFi — ${formatStake(pool.total_stake, pool.token)} staked.`;
  const url = `${SITE_URL}/user/pool-market/${pool.pool_id}`;
  const image = `${SITE_URL}/api/og/pool/${pool.pool_id}`;

  return {
    title,
    description,
    openGraph: {
      title,
      description,
      url,
      siteName: "PrediFi",
      images: [{ url: image, width: 1200, height: 630, alt: pool.name }],
      locale: "en_US",
      type: "website",
    },
    twitter: {
      card: "summary_large_image",
      title,
      description,
      images: [image],
    },
  };
}

export default async function PoolDetailPage({ params }: PageProps) {
  const { id } = await params;

  // Fetch on the server so the happy path paints immediately and keeps SEO
  // metadata. A failure is not fatal: PoolDetailClient owns the error/retry
  // states and will refetch on the client, so we hand it `null` instead of
  // calling notFound() (which would skip retry entirely).
  const pool = await fetchPoolById(id).catch(() => null);

  return <PoolDetailClient poolId={id} initialPool={pool} />;
}

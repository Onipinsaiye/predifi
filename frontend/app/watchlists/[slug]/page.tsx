import type { Metadata } from "next";
import { PublicWatchlistView } from "./PublicWatchlistView";

export const metadata: Metadata = {
  title: "Shared Watchlist | PrediFi",
  description: "A curated collection of prediction markets shared on PrediFi.",
  openGraph: {
    title: "Shared Watchlist | PrediFi",
    description: "A curated collection of prediction markets shared on PrediFi.",
    siteName: "PrediFi",
    images: [{ url: "https://predifi.app/logo.jpeg", width: 1200, height: 630, alt: "PrediFi" }],
    locale: "en_US",
    type: "website",
  },
};

interface PageProps {
  params: Promise<{ slug: string }>;
}

export default async function PublicWatchlistPage({ params }: PageProps) {
  const { slug } = await params;
  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <div className="max-w-3xl mx-auto space-y-8">
        <div className="space-y-1">
          <h1 className="text-3xl font-bold text-white">Shared Watchlist</h1>
          <p className="text-zinc-400 text-sm">
            A curated collection of prediction markets.
          </p>
        </div>
        <PublicWatchlistView slug={slug} />
      </div>
    </div>
  );
}

import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Pool Market",
  description:
    "Explore all prediction pools on PrediFi — browse open and upcoming markets, compare odds, and stake XLM on outcomes powered by Soroban smart contracts.",
  openGraph: {
    title: "Pool Market | PrediFi — Browse All Prediction Pools",
    description:
      "Explore all prediction pools on PrediFi — browse open and upcoming markets, compare odds, and stake XLM on outcomes powered by Soroban smart contracts.",
    url: "https://predifi.app/user/pool-market",
    siteName: "PrediFi",
    images: [
      {
        url: "https://predifi.app/logo.jpeg",
        width: 1200,
        height: 630,
        alt: "PrediFi Pool Market",
      },
    ],
    locale: "en_US",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Pool Market | PrediFi — Browse All Prediction Pools",
    description:
      "Explore all prediction pools on PrediFi — browse open and upcoming markets, compare odds, and stake XLM on outcomes powered by Soroban smart contracts.",
    images: ["https://predifi.app/logo.jpeg"],
  },
};

import { PoolMarketClient } from "./PoolMarketClient";

export default function Page() {
  return (
    <div className="flex flex-col gap-4">
      <div
        role="note"
        aria-label="Demo mode notice"
        className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900"
      >
        <p className="font-semibold">Demo mode available</p>
        <p>
          You can explore the interface before connecting a wallet. Demo values
          are illustrative only and are not live market data. Demo interactions
          stay on this device, and wallet signing is disabled while demo mode is
          active.
        </p>
      </div>
      <PoolMarketClient />
    </div>
  );
}
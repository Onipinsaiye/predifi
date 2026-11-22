import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Creator Dashboard",
  description:
    "Manage the prediction markets you've created on PrediFi — track volume, monitor resolution status, and view reward eligibility across all your authored markets.",
  openGraph: {
    title: "Creator Dashboard | PrediFi",
    description:
      "Manage the prediction markets you've created on PrediFi — track volume, monitor resolution status, and view reward eligibility across all your authored markets.",
    url: "https://predifi.app/user/creator-dashboard",
    siteName: "PrediFi",
    images: [
      {
        url: "https://predifi.app/logo.jpeg",
        width: 1200,
        height: 630,
        alt: "PrediFi Creator Dashboard",
      },
    ],
    locale: "en_US",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Creator Dashboard | PrediFi",
    description:
      "Manage the prediction markets you've created on PrediFi — track volume, monitor resolution status, and view reward eligibility across all your authored markets.",
    images: ["https://predifi.app/logo.jpeg"],
  },
};

import { CreatorDashboardClient } from "./CreatorDashboardClient";

export default function CreatorDashboardPage() {
  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <div className="max-w-5xl mx-auto space-y-8">
        {/* Header */}
        <div className="space-y-1">
          <h1 className="text-3xl font-bold text-white">Creator Dashboard</h1>
          <p className="text-zinc-400 text-sm">
            All the prediction markets you've created — active, closed, and settled.
          </p>
        </div>

        <CreatorDashboardClient />
      </div>
    </div>
  );
}

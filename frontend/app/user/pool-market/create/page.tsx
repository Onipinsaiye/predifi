import type { Metadata } from "next";
import { CreatePoolForm } from "@/components/pool/CreatePoolForm";

export const metadata: Metadata = {
  title: "Create Prediction Pool",
  description:
    "Launch your own prediction market pool on PrediFi — set the question, define outcomes, configure stake limits, and go live on the Stellar network.",
  openGraph: {
    title: "Create Pool | PrediFi — Launch a Prediction Market",
    description:
      "Launch your own prediction market pool on PrediFi — set the question, define outcomes, configure stake limits, and go live on the Stellar network.",
    url: "https://predifi.app/user/pool-market/create",
    siteName: "PrediFi",
    images: [
      {
        url: "https://predifi.app/logo.jpeg",
        width: 1200,
        height: 630,
        alt: "PrediFi Create Pool",
      },
    ],
    locale: "en_US",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Create Pool | PrediFi — Launch a Prediction Market",
    description:
      "Launch your own prediction market pool on PrediFi — set the question, define outcomes, configure stake limits, and go live on the Stellar network.",
    images: ["https://predifi.app/logo.jpeg"],
  },
};

export default function CreatePoolPage() {
  return (
    <div className="min-h-screen bg-[#0A0A0A] px-3 py-4 sm:px-6 sm:py-6 lg:p-8 overflow-x-hidden">
      <div className="mx-auto w-full max-w-2xl space-y-4 sm:space-y-6">
        <div className="space-y-1 px-0.5">
          <h1 className="text-2xl sm:text-3xl font-bold text-white leading-tight">
            Create a Pool
          </h1>
          <p className="text-zinc-400 text-sm leading-relaxed">
            Define a prediction market and invite others to stake on the
            outcome.
          </p>
        </div>

        <CreatePoolForm />
      </div>
    </div>
  );
}

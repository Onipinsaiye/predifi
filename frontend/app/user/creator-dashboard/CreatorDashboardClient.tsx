"use client";

/**
 * Creator dashboard client wrapper (Issue #279).
 *
 * Reads the authenticated wallet address from context and renders the
 * `CreatorDashboard` component, which fetches creator stats and authored
 * markets. Shows a prompt to connect a wallet when no address is available.
 */

import { CreatorDashboard } from "@/components/dashboard/CreatorDashboard";
import { useCurrentUserAddress } from "@/lib/hooks/useCurrentUserAddress";

export function CreatorDashboardClient() {
  const address = useCurrentUserAddress();

  if (!address) {
    return (
      <div
        role="status"
        className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-8 text-center space-y-3"
      >
        <p className="text-white font-semibold">Connect your wallet</p>
        <p className="text-zinc-500 text-sm">
          Connect a Stellar wallet to view the markets you've created.
        </p>
      </div>
    );
  }

  return <CreatorDashboard address={address} />;
}

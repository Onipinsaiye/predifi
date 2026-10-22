"use client";

import useSWR from "swr";
import { fetchProfile, profileUrl, type UserProfile } from "@/lib/api/profile";

export interface UseProfileResult {
  profile: UserProfile | undefined;
  isLoading: boolean;
  isError: boolean;
  /** True when serving cached profile data despite a failing request. */
  isStale: boolean;
  error: Error | undefined;
  refresh: () => void;
}

/**
 * useProfile — SWR-cached access to a user's full profile: aggregate stats,
 * per-pool claim status, and the daily activity series used for charts.
 *
 * The cache key includes both the address and the network so a profile
 * fetched on one network is never reused after a network switch. While the
 * identity is transitioning (no address), no data is returned to avoid
 * flashing the previous user's profile.
 */
export function useProfile(
  address: string | undefined,
  network: string | undefined,
): UseProfileResult {
  const key = address && network ? profileUrl(address, network) : null;

  const { data, error, isLoading, mutate } = useSWR<UserProfile>(key, fetchProfile);

  return {
    profile: key ? data : undefined,
    isLoading,
    isError: Boolean(error),
    isStale: Boolean(error && data),
    error: error as Error | undefined,
    refresh: () => {
      void mutate();
    },
  };
}
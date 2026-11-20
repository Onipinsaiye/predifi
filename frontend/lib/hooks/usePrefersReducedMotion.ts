"use client";

import { useSyncExternalStore } from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

function subscribe(callback: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) {
    return () => {};
  }
  const mediaQueryList = window.matchMedia(QUERY);
  // Modern browsers support addEventListener, legacy fallback to addListener
  if (mediaQueryList.addEventListener) {
    mediaQueryList.addEventListener("change", callback);
    return () => mediaQueryList.removeEventListener("change", callback);
  } else {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (mediaQueryList as any).addListener(callback);
    return () => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (mediaQueryList as any).removeListener(callback);
    };
  }
}

function getSnapshot(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) {
    return false;
  }
  return window.matchMedia(QUERY).matches;
}

function getServerSnapshot(): boolean {
  return false;
}

/**
 * Hook that listens to the operating system's `prefers-reduced-motion` setting.
 * Returns true if the user has requested reduced motion.
 */
export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

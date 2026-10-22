import React from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { AnimatedNumber } from "@/components/pool/AnimatedNumber";
import { usePrefersReducedMotion } from "@/lib/hooks/usePrefersReducedMotion";
import { renderHook, act } from "@testing-library/react";

describe("usePrefersReducedMotion", () => {
  let listeners: Array<(e: MediaQueryListEvent | MediaQueryList) => void> = [];
  let matches = false;

  beforeEach(() => {
    listeners = [];
    matches = false;
    window.matchMedia = jest.fn().mockImplementation((query) => ({
      matches,
      media: query,
      onchange: null,
      addListener: (cb: any) => listeners.push(cb),
      removeListener: (cb: any) => {
        listeners = listeners.filter((l) => l !== cb);
      },
      addEventListener: (_ev: string, cb: any) => listeners.push(cb),
      removeEventListener: (_ev: string, cb: any) => {
        listeners = listeners.filter((l) => l !== cb);
      },
      dispatchEvent: () => false,
    }));
  });

  it("returns false by default when prefers-reduced-motion is false", () => {
    const { result } = renderHook(() => usePrefersReducedMotion());
    expect(result.current).toBe(false);
  });

  it("returns true when prefers-reduced-motion is true", () => {
    matches = true;
    const { result } = renderHook(() => usePrefersReducedMotion());
    expect(result.current).toBe(true);
  });

  it("updates reactively when media query matches change", () => {
    const { result } = renderHook(() => usePrefersReducedMotion());
    expect(result.current).toBe(false);

    act(() => {
      matches = true;
      listeners.forEach((l) => l({ matches: true } as any));
    });

    expect(result.current).toBe(true);
  });
});

describe("AnimatedNumber reduced motion support", () => {
  let matches = false;

  beforeEach(() => {
    matches = false;
    window.matchMedia = jest.fn().mockImplementation((query) => ({
      matches,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));
  });

  it("immediately displays updated value without waiting for RAF duration when reduced motion is requested", () => {
    matches = true;
    const { rerender } = render(<AnimatedNumber value={100} durationMs={2000} />);
    expect(screen.getByText("100")).toBeInTheDocument();

    rerender(<AnimatedNumber value={500} durationMs={2000} />);
    // With reduced motion, should immediately be 500
    expect(screen.getByText("500")).toBeInTheDocument();
  });
});

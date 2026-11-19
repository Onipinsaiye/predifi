import { render, screen } from "@testing-library/react";
import { getFreshness } from "@/lib/freshness";
import { FreshnessIndicator } from "@/components/common/FreshnessIndicator";

describe("getFreshness", () => {
  const now = Date.parse("2026-01-01T00:00:00Z");

  it("is unavailable when no timestamp is reported", () => {
    expect(getFreshness(null, now).state).toBe("unavailable");
    expect(getFreshness(undefined, now).state).toBe("unavailable");
    expect(getFreshness("not-a-date", now).state).toBe("unavailable");
  });

  it("is fresh within the staleness bound", () => {
    const indexedAt = new Date(now - 10_000).toISOString();
    const info = getFreshness(indexedAt, now, 60_000);
    expect(info.state).toBe("fresh");
    expect(info.ageMs).toBe(10_000);
  });

  it("is stale beyond the staleness bound", () => {
    const indexedAt = new Date(now - 120_000).toISOString();
    const info = getFreshness(indexedAt, now, 60_000);
    expect(info.state).toBe("stale");
    expect(info.ageMs).toBe(120_000);
  });
});

describe("FreshnessIndicator", () => {
  const now = Date.parse("2026-01-01T00:00:00Z");

  beforeEach(() => {
    jest.spyOn(Date, "now").mockReturnValue(now);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("renders an unknown state when indexedAt is unavailable", () => {
    render(<FreshnessIndicator indexedAt={null} />);
    expect(screen.getByText("Freshness unknown")).toBeInTheDocument();
  });

  it("renders a stale warning when lagging behind the ledger", () => {
    render(<FreshnessIndicator indexedAt={new Date(now - 120_000).toISOString()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Behind ledger");
  });

  it("renders a quiet updated indicator when fresh", () => {
    render(<FreshnessIndicator indexedAt={new Date(now - 5_000).toISOString()} />);
    expect(screen.getByText(/Updated/)).toBeInTheDocument();
  });
});

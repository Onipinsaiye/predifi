/**
 * Pool list route — loading / success / empty / error→retry / partial (#84).
 *
 * Exercises the real `PoolMarketClient` against shared API fixtures so the
 * five states the route can be in are pinned down.
 */

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PoolMarketClient } from "@/app/user/pool-market/PoolMarketClient";
import { poolsUrl } from "@/lib/api/pools";
import { renderRoute } from "../test-utils/renderRoute";
import {
  TEST_POOL,
  TEST_POOL_TWO,
  EMPTY_POOLS_RESPONSE,
  deferred,
  errorResponse,
  jsonResponse,
  poolsResponse,
  createApiFetchMock,
} from "../test-utils/apiFixtures";

const POOLS_KEY = poolsUrl({
  sort_by: "new",
  status: "active",
  limit: 20,
  offset: 0,
});

describe("Pool list route states", () => {
  it("shows an accessible loading state before the first response", async () => {
    const pending = deferred<Response>();
    createApiFetchMock({ pools: () => pending.promise });

    renderRoute(<PoolMarketClient />);

    expect(screen.getByText("Loading prediction pools")).toBeInTheDocument();
    expect(screen.getByText("Loading pools…")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Pool Market",
    );
  });

  it("renders pools and a meaningful count once loading succeeds", async () => {
    createApiFetchMock({
      pools: () => jsonResponse(poolsResponse([TEST_POOL, TEST_POOL_TWO])),
    });

    renderRoute(<PoolMarketClient />);

    expect(
      await screen.findByText("Will BTC close above $100k?"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Will it rain in Lagos on Friday?"),
    ).toBeInTheDocument();
    expect(screen.getByText("2 pools found")).toBeInTheDocument();
  });

  it("shows a route-specific empty state when no pools match the filters", async () => {
    createApiFetchMock({ pools: () => jsonResponse(EMPTY_POOLS_RESPONSE) });

    renderRoute(<PoolMarketClient />);

    const emptyHeading = await screen.findByRole("heading", {
      name: "No pools found for these filters",
    });
    expect(emptyHeading).toBeInTheDocument();
    expect(
      screen.getByText(/Adjust the category, tag, or status filters/),
    ).toBeInTheDocument();
    // Empty is a polite status; the error heading must not be present.
    expect(
      screen.queryByRole("heading", { name: /couldn't load pools/i }),
    ).not.toBeInTheDocument();
    expect(emptyHeading.closest("[role='status']")).not.toBeNull();
  });

  it("shows an alert error state and recovers when retry is pressed", async () => {
    const user = userEvent.setup();
    let attempt = 0;
    createApiFetchMock({
      pools: () => {
        attempt += 1;
        return attempt === 1
          ? errorResponse(500)
          : jsonResponse(poolsResponse([TEST_POOL]));
      },
    });

    renderRoute(<PoolMarketClient />);

    const errorHeading = await screen.findByRole("heading", {
      name: "Couldn't load pools",
    });
    // The error surface is an assertive alert region.
    expect(errorHeading.closest("[role='alert']")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: /try again/i }));

    expect(
      await screen.findByText("Will BTC close above $100k?"),
    ).toBeInTheDocument();
    expect(attempt).toBeGreaterThanOrEqual(2);
  });

  it("keeps showing cached pools with a partial-data notice when the API fails", async () => {
    createApiFetchMock({ pools: () => errorResponse(503) });

    renderRoute(<PoolMarketClient />, {
      swrFallback: { [POOLS_KEY]: poolsResponse([TEST_POOL]) },
    });

    expect(
      await screen.findByText(/Showing cached results/),
    ).toBeInTheDocument();
    expect(screen.getByText("Will BTC close above $100k?")).toBeInTheDocument();
    // Stale data is a polite status, not an error state.
    expect(
      screen.queryByRole("heading", { name: /couldn't load pools/i }),
    ).not.toBeInTheDocument();
  });
});

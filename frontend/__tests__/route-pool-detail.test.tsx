/**
 * Pool detail route — loading / success / empty(404) / error→retry / partial (#84).
 *
 * `PoolDetailClient` owns the client-side states; the server page hands it an
 * optional `initialPool` for the first paint. The error boundary is covered
 * separately by rendering the route's `error.tsx` body.
 */

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PoolDetailClient } from "@/app/user/pool-market/[id]/PoolDetailClient";
import PoolDetailError from "@/app/user/pool-market/[id]/error";
import { poolDetailUrl } from "@/lib/api/pools";
import { renderRoute } from "../test-utils/renderRoute";
import {
  TEST_POOL_DETAIL,
  TEST_POOL_DETAIL_NO_ODDS,
  deferred,
  errorResponse,
  jsonResponse,
  createApiFetchMock,
} from "../test-utils/apiFixtures";

const POOL_ID = "1";
const DETAIL_KEY = poolDetailUrl(POOL_ID);

describe("Pool detail route states", () => {
  it("shows an accessible loading state while the pool request is in flight", async () => {
    const pending = deferred<Response>();
    createApiFetchMock({ poolDetail: () => pending.promise });

    renderRoute(<PoolDetailClient poolId={POOL_ID} />);

    const loadingLabel = screen.getByText("Loading pool details");
    const status = loadingLabel.closest("[role='status']");
    expect(status).not.toBeNull();
    expect(status).toHaveAttribute("aria-busy", "true");
  });

  it("renders the pool heading, outcomes, and odds on success", async () => {
    createApiFetchMock({ poolDetail: () => jsonResponse(TEST_POOL_DETAIL) });

    renderRoute(<PoolDetailClient poolId={POOL_ID} />);

    expect(
      await screen.findByRole("heading", {
        level: 1,
        name: "Will BTC close above $100k?",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /Outcomes & Odds/i }),
    ).toBeInTheDocument();
    expect(screen.getByText("Yes")).toBeInTheDocument();
    expect(screen.getByText("No")).toBeInTheDocument();
    expect(screen.getByText("1.56x")).toBeInTheDocument();
  });

  it("renders the server-provided pool immediately without a loading flash", async () => {
    createApiFetchMock({ poolDetail: () => jsonResponse(TEST_POOL_DETAIL) });

    renderRoute(
      <PoolDetailClient poolId={POOL_ID} initialPool={TEST_POOL_DETAIL} />,
    );

    // Present on the very first paint — no `findBy*` wait required.
    expect(
      screen.getByRole("heading", {
        level: 1,
        name: "Will BTC close above $100k?",
      }),
    ).toBeInTheDocument();
  });

  it("shows a not-found empty state (no retry) for a missing pool", async () => {
    createApiFetchMock({ poolDetail: () => errorResponse(404) });

    renderRoute(<PoolDetailClient poolId="999" />);

    expect(
      await screen.findByRole("heading", { name: "Pool not found" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: /back to pools/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /try again/i }),
    ).not.toBeInTheDocument();
  });

  it("shows an error state and retries the pool request", async () => {
    const user = userEvent.setup();
    let attempt = 0;
    createApiFetchMock({
      poolDetail: () => {
        attempt += 1;
        return attempt === 1
          ? errorResponse(500)
          : jsonResponse(TEST_POOL_DETAIL);
      },
    });

    renderRoute(<PoolDetailClient poolId={POOL_ID} />);

    const errorHeading = await screen.findByRole("heading", {
      name: "Couldn't load this pool",
    });
    expect(errorHeading.closest("[role='alert']")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: /try again/i }));

    expect(
      await screen.findByRole("heading", {
        level: 1,
        name: "Will BTC close above $100k?",
      }),
    ).toBeInTheDocument();
  });

  it("shows a partial-data notice when a cached pool is served during an outage", async () => {
    createApiFetchMock({ poolDetail: () => errorResponse(503) });

    renderRoute(<PoolDetailClient poolId={POOL_ID} />, {
      swrFallback: { [DETAIL_KEY]: TEST_POOL_DETAIL },
    });

    expect(
      await screen.findByText(/Showing cached pool data/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", {
        level: 1,
        name: "Will BTC close above $100k?",
      }),
    ).toBeInTheDocument();
  });

  it("flags a pool with no published outcomes instead of rendering an empty list", async () => {
    createApiFetchMock({
      poolDetail: () => jsonResponse(TEST_POOL_DETAIL_NO_ODDS),
    });

    renderRoute(<PoolDetailClient poolId={POOL_ID} />);

    expect(
      await screen.findByText(/No outcomes are published for this pool yet/),
    ).toBeInTheDocument();
    // The heading still exists, so the page outline stays meaningful.
    expect(
      screen.getByRole("heading", { name: /Outcomes & Odds/i }),
    ).toBeInTheDocument();
  });

  it("error boundary renders an alert with retry and a back link", async () => {
    const reset = jest.fn();
    renderRoute(<PoolDetailError error={new Error("boom")} reset={reset} />);

    const heading = screen.getByRole("heading", {
      name: "Failed to load pool",
    });
    expect(heading.closest("[role='alert']")).not.toBeNull();
    expect(
      screen.getByRole("link", { name: /back to pools/i }),
    ).toHaveAttribute("href", "/user/pool-market");

    await userEvent.click(screen.getByRole("button", { name: /try again/i }));
    expect(reset).toHaveBeenCalledTimes(1);
  });
});

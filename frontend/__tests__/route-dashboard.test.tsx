/**
 * Dashboard route — loading / success / empty / error→retry / partial (#84).
 *
 * The dashboard composes several pieces: metric cards (`DashboardMetrics`),
 * the active-pools list (`PoolsList`), and route-level `loading.tsx` /
 * `error.tsx` boundaries. These tests exercise each of the five states.
 */

import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import DashboardPage from "@/app/dashboard/page";
import DashboardLoading from "@/app/dashboard/loading";
import DashboardError from "@/app/dashboard/error";
import { PoolsList } from "@/components/dashboard/PoolsList";
import { poolsUrl } from "@/lib/api/pools";
import { renderRoute } from "../test-utils/renderRoute";
import {
  TEST_POOL,
  EMPTY_POOLS_RESPONSE,
  deferred,
  errorResponse,
  installResizeObserverMock,
  jsonResponse,
  poolsResponse,
  createApiFetchMock,
} from "../test-utils/apiFixtures";

const ACTIVE_POOLS_KEY = poolsUrl({ status: "active", sort_by: "new" });

beforeAll(() => {
  installResizeObserverMock();
});

describe("Dashboard route states", () => {
  it("shows metric skeletons while the pools request is in flight", async () => {
    const pending = deferred<Response>();
    createApiFetchMock({ pools: () => pending.promise });

    renderRoute(<DashboardPage />);

    // The metric row announces it is busy without rendering fake values.
    const metricsRow = document.querySelector(
      '[role="status"][aria-busy="true"]',
    );
    expect(metricsRow).not.toBeNull();
    expect(screen.queryByText("Total Earned")).not.toBeInTheDocument();
  });

  it("renders metrics and the active pool list on success", async () => {
    createApiFetchMock({ pools: () => jsonResponse(poolsResponse([TEST_POOL])) });

    renderRoute(<DashboardPage />);

    expect(await screen.findByText("Total Earned")).toBeInTheDocument();
    expect(screen.getAllByText("Active Pools").length).toBeGreaterThan(0);
    expect(
      screen.getByRole("heading", { level: 1, name: "My Dashboard" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Will BTC close above $100k?"),
    ).toBeInTheDocument();
  });

  it("shows route-specific empty states when there are no active pools", async () => {
    createApiFetchMock({ pools: () => jsonResponse(EMPTY_POOLS_RESPONSE) });

    renderRoute(<DashboardPage />);

    expect(
      await screen.findByRole("heading", { name: "No active pools yet" }),
    ).toBeInTheDocument();
    // The dashboard heading is preserved through the empty state.
    expect(
      screen.getByRole("heading", { level: 1, name: "My Dashboard" }),
    ).toBeInTheDocument();
  });

  it("shows an error state and recovers via retry", async () => {
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

    renderRoute(<DashboardPage />);

    const errorHeading = await screen.findByRole("heading", {
      name: "Couldn't load your dashboard metrics",
    });
    const alert = errorHeading.closest("[role='alert']");
    expect(alert).not.toBeNull();

    const retry = (alert as HTMLElement).querySelector("button");
    expect(retry).not.toBeNull();
    await user.click(retry as HTMLButtonElement);

    expect(await screen.findByText("Total Earned")).toBeInTheDocument();
    expect(attempt).toBeGreaterThanOrEqual(2);
  });

  it("keeps cached metrics and pools with a partial-data notice during an outage", async () => {
    createApiFetchMock({ pools: () => errorResponse(503) });

    renderRoute(<DashboardPage />, {
      swrFallback: { [ACTIVE_POOLS_KEY]: poolsResponse([TEST_POOL]) },
    });

    expect(
      await screen.findByText(/Showing cached metrics/),
    ).toBeInTheDocument();
    expect(screen.getByText("Total Earned")).toBeInTheDocument();
    expect(screen.getByText("Will BTC close above $100k?")).toBeInTheDocument();
  });

  it("route loading UI mirrors the dashboard layout accessibly", () => {
    renderRoute(<DashboardLoading />);

    expect(screen.getByText("Loading statistics")).toBeInTheDocument();
    expect(screen.getByText("Loading active pools")).toBeInTheDocument();
    expect(
      screen.getByText("Loading recent predictions"),
    ).toBeInTheDocument();
  });

  it("route error boundary renders an alert with retry and a back link", async () => {
    const reset = jest.fn();
    renderRoute(<DashboardError error={new Error("boom")} reset={reset} />);

    const heading = screen.getByRole("heading", {
      name: "Failed to load your dashboard",
    });
    expect(heading.closest("[role='alert']")).not.toBeNull();
    expect(
      screen.getByRole("link", { name: /back to pools/i }),
    ).toHaveAttribute("href", "/user/pool-market");

    await userEvent.click(screen.getByRole("button", { name: /try again/i }));
    expect(reset).toHaveBeenCalledTimes(1);
  });
});

describe("Dashboard PoolsList component states", () => {
  it("shows an inline error state with retry when the pool fetch fails", async () => {
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

    renderRoute(<PoolsList />);

    const errorHeading = await screen.findByRole("heading", {
      name: "Couldn't load active pools",
    });
    expect(errorHeading.closest("[role='alert']")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: /try again/i }));

    expect(
      await screen.findByText("Will BTC close above $100k?"),
    ).toBeInTheDocument();
  });

  it("shows a search empty state when the query matches nothing", async () => {
    const user = userEvent.setup();
    createApiFetchMock({ pools: () => jsonResponse(poolsResponse([TEST_POOL])) });

    renderRoute(<PoolsList />);

    await screen.findByText("Will BTC close above $100k?");
    await user.type(screen.getByRole("searchbox"), "zzz");

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "No pools match your search" }),
      ).toBeInTheDocument(),
    );
  });
});

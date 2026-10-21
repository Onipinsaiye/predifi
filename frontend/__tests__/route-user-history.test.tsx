/**
 * User history route — loading / success / empty / error→retry / partial (#84).
 *
 * Covers `PredictionHistoryClient`, which drives the profile + claim history
 * from `useProfile`, and the route's `error.tsx` boundary.
 */

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PredictionHistoryClient } from "@/app/user/my-profile/PredictionHistoryClient";
import MyProfileError from "@/app/user/my-profile/error";
import { profileUrl } from "@/lib/api/profile";
import { renderRoute } from "../test-utils/renderRoute";
import {
  EMPTY_PROFILE,
  TEST_ADDRESS,
  TEST_CLAIM,
  deferred,
  errorResponse,
  jsonResponse,
  profileFixture,
  createApiFetchMock,
} from "../test-utils/apiFixtures";

const PROFILE_KEY = profileUrl(TEST_ADDRESS);

describe("User history route states", () => {
  it("shows accessible loading skeletons for stats and history", async () => {
    const pending = deferred<Response>();
    createApiFetchMock({ profile: () => pending.promise });

    renderRoute(<PredictionHistoryClient />);

    expect(screen.getByText("Loading your statistics")).toBeInTheDocument();
    expect(screen.getByText("Loading prediction history")).toBeInTheDocument();
    // The page heading survives the loading state.
    expect(
      screen.getByRole("heading", { level: 1, name: "My Profile" }),
    ).toBeInTheDocument();
  });

  it("renders stats and prediction rows once loading succeeds", async () => {
    createApiFetchMock({ profile: () => jsonResponse(profileFixture()) });

    renderRoute(<PredictionHistoryClient />);

    expect(await screen.findByText("Total Predictions")).toBeInTheDocument();
    expect(screen.getByText("Will BTC close above $100k?")).toBeInTheDocument();
    expect(
      screen.getByText("Will it rain in Lagos on Friday?"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Prediction History" }),
    ).toBeInTheDocument();
  });

  it("shows a route-specific empty state when there is no history", async () => {
    createApiFetchMock({ profile: () => jsonResponse(EMPTY_PROFILE) });

    renderRoute(<PredictionHistoryClient />);

    const emptyHeading = await screen.findByRole("heading", {
      name: "No predictions yet",
    });
    expect(emptyHeading.closest("[role='status']")).not.toBeNull();
    expect(
      screen.queryByRole("heading", { name: /couldn't load your history/i }),
    ).not.toBeInTheDocument();
  });

  it("shows an error state and retries the profile request", async () => {
    const user = userEvent.setup();
    let attempt = 0;
    createApiFetchMock({
      profile: () => {
        attempt += 1;
        return attempt === 1
          ? errorResponse(500)
          : jsonResponse(profileFixture());
      },
    });

    renderRoute(<PredictionHistoryClient />);

    const errorHeading = await screen.findByRole("heading", {
      name: "Couldn't load your history",
    });
    expect(errorHeading.closest("[role='alert']")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: /try again/i }));

    expect(await screen.findByText("Total Predictions")).toBeInTheDocument();
    expect(attempt).toBeGreaterThanOrEqual(2);
  });

  it("keeps cached history visible with a partial-data notice during an outage", async () => {
    createApiFetchMock({ profile: () => errorResponse(503) });

    renderRoute(<PredictionHistoryClient />, {
      swrFallback: { [PROFILE_KEY]: profileFixture() },
    });

    expect(
      await screen.findByText(/Showing cached history/),
    ).toBeInTheDocument();
    expect(screen.getByText("Will BTC close above $100k?")).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: /couldn't load your history/i }),
    ).not.toBeInTheDocument();
  });

  it("shows a filter-specific empty state when a tab has no matching rows", async () => {
    const user = userEvent.setup();
    // Only an active/pending claim exists, so the "Won" tab is empty.
    createApiFetchMock({
      profile: () =>
        jsonResponse(profileFixture({ claims: [TEST_CLAIM] })),
    });

    renderRoute(<PredictionHistoryClient />);

    await screen.findByText("Will BTC close above $100k?");
    await user.click(screen.getByRole("tab", { name: "Lost" }));

    expect(
      await screen.findByRole("heading", { name: "No lost predictions" }),
    ).toBeInTheDocument();
  });

  it("error boundary renders an alert with retry and a back link", async () => {
    const reset = jest.fn();
    renderRoute(<MyProfileError error={new Error("boom")} reset={reset} />);

    const heading = screen.getByRole("heading", {
      name: "Failed to load your history",
    });
    expect(heading.closest("[role='alert']")).not.toBeNull();
    expect(
      screen.getByRole("link", { name: /back to pools/i }),
    ).toHaveAttribute("href", "/user/pool-market");

    await userEvent.click(screen.getByRole("button", { name: /try again/i }));
    expect(reset).toHaveBeenCalledTimes(1);
  });
});

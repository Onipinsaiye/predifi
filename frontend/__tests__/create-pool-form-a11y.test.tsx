
/**
 * Accessibility + keyboard-only flow tests for CreatePoolForm.
 *
 * Covers:
 * - axe automated accessibility scan on initial render
 * - axe scan after validation errors appear
 * - axe scan on the success state
 * - keyboard-only flow: Tab through all fields, add/remove outcomes,
 *   trigger validation, fix errors, and complete a valid submission
 */
import React from "react";
import { render, screen, within, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { CreatePoolForm } from "@/components/pool/CreatePoolForm";

// ── Mock next/navigation (used by the SuccessState) ──────────────────────────
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn(), forward: jest.fn() }),
}));

// ── Helpers ──────────────────────────────────────────────────────────────────

/** A future datetime-local string that satisfies the "at least 1 hour" rule. */
function validCloseTime(): string {
  const d = new Date(Date.now() + 3600 * 1000 * 48); // 48 hours from now
  return d.toISOString().slice(0, 16);
}

/** Returns the live region used to announce status/validation changes. */
function getStatusRegion(): HTMLElement {
  return screen.getByRole("status");
}

/** Returns the alert region used to announce validation errors. */
function getAlertRegion(): HTMLElement {
  return screen.getByRole("alert");
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("CreatePoolForm accessibility", () => {
  it("has no axe violations on initial render", async () => {
    const { container } = render(<CreatePoolForm />);
    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });

  it("exposes a polite live region for status updates", () => {
    render(<CreatePoolForm />);
    const status = getStatusRegion();
    expect(status).toHaveAttribute("aria-live", "polite");
    expect(status).toHaveAttribute("aria-atomic", "true");
  });

  it("has no axe violations after validation errors appear", async () => {
    const { container } = render(<CreatePoolForm />);

    // Submit empty form to trigger all validation errors
    const submitButton = screen.getByRole("button", { name: /create pool/i });
    await userEvent.click(submitButton);

    const outcome1 = screen.getByLabelText("Outcome 1");
    expect(outcome1).toHaveAttribute("aria-invalid", "true");
    expect(outcome1).toHaveAccessibleDescription("Outcome label is required.");

    // Validation errors must be announced via an assertive alert region.
    const alert = getAlertRegion();
    expect(alert).toHaveTextContent(/outcome label is required/i);

    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });

  it("has no axe violations on the success state", async () => {
    const { container } = render(<CreatePoolForm />);

    // Fill all required fields via keyboard
    const user = userEvent.setup();

    await user.type(screen.getByLabelText(/pool name/i), "Will BTC reach $100K by end of year?");
    await user.type(
      screen.getByLabelText(/description/i),
      "Prediction market pool for BTC price action in 2026.",
    );

    // Category select
    const categorySelect = screen.getByLabelText(/category/i);
    await user.selectOptions(categorySelect, "Crypto");

    // Outcomes (already have 2 by default)
    const outcome1 = screen.getByLabelText("Outcome 1");
    const outcome2 = screen.getByLabelText("Outcome 2");
    await user.type(outcome1, "Yes");
    await user.type(outcome2, "No");

    // Stake fields
    await user.type(screen.getByLabelText(/minimum stake/i), "5");
    await user.type(screen.getByLabelText(/maximum stake/i), "500");

    // Close time
    await user.type(screen.getByLabelText(/close time/i), validCloseTime());

    // Terms checkbox
    const termsCheckbox = screen.getByRole("checkbox", {
      name: /smart contracts on the stellar network/i,
    });
    termsCheckbox.focus();
    await user.keyboard(" ");

    // Submit
    const submitButton = screen.getByRole("button", { name: /create pool/i });
    submitButton.focus();
    await user.keyboard("{Enter}");

    // Wait for the success state to appear (setTimeout 1500ms in component)
    const successHeading = await screen.findByRole(
      "heading",
      {
        name: /pool created successfully/i,
      },
      { timeout: 3000 },
    );
    expect(successHeading).toBeInTheDocument();

    // Success must be announced through the status live region.
    await waitFor(() => {
      expect(getStatusRegion()).toHaveTextContent(/pool created successfully/i);
    });

    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });
});

// ── Keyboard-only flow tests ─────────────────────────────────────────────────

describe("CreatePoolForm keyboard-only flow", () => {
  it("allows reaching all interactive elements via Tab in logical order", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Tab to the first input — "Pool Name"
    await user.tab();
    expect(screen.getByLabelText(/pool name/i)).toHaveFocus();

    // Tab through to Description textarea
    await user.tab();
    expect(screen.getByLabelText(/description/i)).toHaveFocus();

    // Tab to Category select
    await user.tab();
    expect(screen.getByLabelText(/category/i)).toHaveFocus();

    // Tab to Tags input
    await user.tab();
    expect(screen.getByLabelText(/tags/i)).toHaveFocus();

    // Tab to Outcome 1
    await user.tab();
    expect(screen.getByLabelText("Outcome 1")).toHaveFocus();

    // Tab to Outcome 2
    await user.tab();
    expect(screen.getByLabelText("Outcome 2")).toHaveFocus();

    // Tab to "Add outcome" button
    await user.tab();
    expect(screen.getByRole("button", { name: /add outcome/i })).toHaveFocus();

    // Continue Tabbing through remaining fields
    await user.tab(); // token picker button
    expect(screen.getByRole("button", { name: /select token/i })).toHaveFocus();

    await user.tab(); // min stake
    expect(screen.getByLabelText(/minimum stake/i)).toHaveFocus();

    await user.tab(); // max stake
    expect(screen.getByLabelText(/maximum stake/i)).toHaveFocus();

    await user.tab(); // close time
    expect(screen.getByLabelText(/close time/i)).toHaveFocus();
  });

  it("moves focus to the new outcome input when Add Outcome is activated", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Focus the Add Outcome button and activate it with keyboard
    const addButton = screen.getByRole("button", { name: /add outcome/i });
    addButton.focus();
    expect(addButton).toHaveFocus();

    await user.keyboard("{Enter}");

    // Focus should move to the newly created Outcome 3
    const outcome3 = screen.getByLabelText("Outcome 3");
    expect(outcome3).toBeInTheDocument();
    expect(outcome3).toHaveFocus();
  });

  it("moves focus to the previous outcome when an outcome is removed", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Add a third outcome
    const addButton = screen.getByRole("button", { name: /add outcome/i });
    addButton.focus();
    await user.keyboard("{Enter}");

    // Remove outcome 2 (the middle one)
    const removeButton = screen.getByRole("button", {
      name: "Remove outcome 2",
    });
    expect(removeButton).toHaveClass("focus-visible:ring-2");
    removeButton.focus();
    expect(removeButton).toHaveFocus();

    await user.keyboard("{Enter}");

    // Focus should land on Outcome 1 (previous in DOM order)
    expect(screen.getByLabelText("Outcome 1")).toHaveFocus();
  });

  it("removes the correct outcome when the third is removed and focus goes to the second", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Fill outcomes so we can verify which was removed
    await user.type(screen.getByLabelText("Outcome 1"), "Yes");
    await user.type(screen.getByLabelText("Outcome 2"), "No");

    // Add a third outcome
    const addButton = screen.getByRole("button", { name: /add outcome/i });
    addButton.focus();
    await user.keyboard("{Enter}");

    // Type into the third outcome
    await user.keyboard("Maybe");

    // Remove the third outcome
    const removeButton = screen.getByRole("button", {
      name: "Remove outcome 3",
    });
    removeButton.focus();
    await user.keyboard("{Enter}");

    // Should have 2 outcomes again
    expect(screen.queryByLabelText("Outcome 3")).not.toBeInTheDocument();

    // Focus should land on Outcome 2 (previous)
    expect(screen.getByLabelText("Outcome 2")).toHaveFocus();
    expect(screen.getByLabelText("Outcome 2")).toHaveValue("No");
  });

  it("associates outcome validation errors and clears them when corrected", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    await user.type(screen.getByLabelText("Outcome 1"), "Yes");
    await user.type(screen.getByLabelText("Outcome 2"), "yes");

    await user.click(screen.getByRole("button", { name: /create pool/i }));

    const outcome2 = screen.getByLabelText("Outcome 2");
    expect(outcome2).toHaveAttribute("aria-invalid", "true");
    expect(outcome2).toHaveAccessibleDescription("Outcome labels must be unique.");

    // The error must be associated and announced.
    expect(getAlertRegion()).toHaveTextContent(/outcome labels must be unique/i);
    expect(outcome2).toHaveAttribute("aria-describedby");

    await user.clear(screen.getByLabelText("Outcome 1"));
    await user.type(screen.getByLabelText("Outcome 1"), "No");

    expect(outcome2).toHaveAttribute("aria-invalid", "false");
    expect(screen.queryByText("Outcome labels must be unique.")).not.toBeInTheDocument();

    // The alert region should be cleared once the error is resolved.
    await waitFor(() => {
      expect(getAlertRegion()).toBeEmptyDOMElement();
    });
  });

  it("focuses the first error field when submitting an invalid form", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Submit empty form
    const submitButton = screen.getByRole("button", { name: /create pool/i });
    submitButton.focus();
    await user.keyboard("{Enter}");

    // First error field should be "Pool Name" — it must have focus
    const nameInput = screen.getByLabelText(/pool name/i);
    expect(nameInput).toHaveFocus();
    expect(nameInput).toHaveAttribute("aria-invalid", "true");

    // Focus must be preserved on the first error field after the announcement.
    expect(getAlertRegion()).toHaveTextContent(/pool name is required/i);
    expect(nameInput).toHaveFocus();
  });

  it("completes a full valid submission using only keyboard", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Tab to Pool Name and type
    await user.tab();
    await user.keyboard("Will BTC reach $100K by end of year?");

    // Tab to Description
    await user.tab();
    await user.keyboard("Prediction market pool for BTC price action in 2026.");

    // Tab to Category. jsdom does not emulate native-select keyboard selection,
    // so use Testing Library's pointer-free native selection helper here.
    await user.tab();
    await user.selectOptions(screen.getByLabelText(/category/i), "Crypto");

    // Tab to Tags (skip)
    await user.tab();

    // Tab to Outcome 1
    await user.tab();
    await user.keyboard("Yes");

    // Tab to Outcome 2
    await user.tab();
    await user.keyboard("No");

    // Tab to Add Outcome button and skip it
    await user.tab();

    // Tab to token picker and skip it
    await user.tab();

    // Tab to Minimum Stake
    await user.tab();
    await user.keyboard("5");

    // Tab to Maximum Stake
    await user.tab();
    await user.keyboard("500");

    // Tab to Close Time
    await user.tab();
    await user.keyboard(validCloseTime());

    // Tab to the terms checkbox. The informational hint is not focusable.
    await user.tab();

    // Terms checkbox — toggle with Space
    const termsCheckbox = screen.getByRole("checkbox", {
      name: /smart contracts on the stellar network/i,
    });
    expect(termsCheckbox).toHaveFocus();
    await user.keyboard(" ");

    // Tab to Reset button and skip it
    await user.tab();

    // Tab to Submit button
    await user.tab();
    const submitButton = screen.getByRole("button", { name: /create pool/i });
    expect(submitButton).toHaveFocus();

    // Submit
    await user.keyboard("{Enter}");

    // Wait for the success state
    const successHeading = await screen.findByRole(
      "heading",
      {
        name: /pool created successfully/i,
      },
      { timeout: 3000 },
    );
    expect(successHeading).toBeInTheDocument();

    // Status announcement is present for screen-reader users.
    await waitFor(() => {
      expect(getStatusRegion()).toHaveTextContent(/pool created successfully/i);
    });
  });

  it("shows validation errors and allows correcting them via keyboard", async () => {
    const user = userEvent.setup();
    render(<CreatePoolForm />);

    // Submit empty form
    const submitButton = screen.getByRole("button", { name: /create pool/i });
    submitButton.focus();
    await user.keyboard("{Enter}");

    // First error: Pool Name
    expect(screen.getByLabelText(/pool name/i)).toHaveFocus();

    // Fix the name
    await user.keyboard("Will BTC reach $100K by end of year?");

    // Resubmit
    submitButton.focus();
    await user.keyboard("{Enter}");

    // Focus should now be on Description (next error)
    expect(screen.getByLabelText(/description/i)).toHaveFocus();
    expect(screen.getByLabelText(/description/i)).toHaveAttribute("aria-invalid", "true");

    // The new error must be announced without moving focus away.
    expect(getAlertRegion()).toHaveTextContent(/description is required/i);
    expect(screen.getByLabelText(/description/i)).toHaveFocus();

    // Fix the description
    await user.keyboard("Prediction market pool for BTC price action in 2026.");

    // Resubmit
    submitButton.focus();
    await user.keyboard("{Enter}");

    // Focus should now be on Category (next error)
    expect(screen.getByLabelText(/category/i)).toHaveFocus();
  });
});

// frontend/lib/lifecycleExplanations.ts

/**
 * Shared localization catalog for market lifecycle state explanations.
 *
 * Each entry provides reviewed copy describing:
 * - what the state means,
 * - what actions remain possible,
 * - the next expected transition,
 * - the authoritative source for that transition.
 *
 * Keep all copy in this single catalog so it can be reviewed and localized
 * in one place. Unknown/future states fall back to a safe generic explanation.
 */

export type LifecycleState =
  | 'open'
  | 'closing_soon'
  | 'closed'
  | 'resolving'
  | 'resolved'
  | 'claimable'
  | 'claimed'
  | 'disputed'
  | 'cancelled';

export interface LifecycleExplanation {
  /** The lifecycle state this explanation describes. */
  state: string;
  /** Human-readable label for the state. */
  label: string;
  /** Reviewed description of what the state means. */
  description: string;
  /** What actions remain possible in this state. */
  remainingActions: string;
  /** The next expected transition, or a description of the terminal state. */
  nextTransition: string;
  /** The authoritative source for the next transition. */
  authoritativeSource: string;
  /** Whether this is a terminal state with no further transitions. */
  terminal: boolean;
}

const CATALOG: Record<LifecycleState, LifecycleExplanation> = {
  open: {
    state: 'open',
    label: 'Open',
    description: 'The market is accepting predictions and participants can take positions.',
    remainingActions: 'You can place or adjust your position until the market closes.',
    nextTransition: 'The market will move to "Closing soon" as the close time approaches.',
    authoritativeSource: 'Market close time recorded on-chain by the market contract.',
    terminal: false,
  },
  closing_soon: {
    state: 'closing_soon',
    label: 'Closing soon',
    description: 'The market is nearing its close time and will stop accepting predictions shortly.',
    remainingActions: 'You can still place or adjust your position, but only briefly.',
    nextTransition: 'The market will move to "Closed" once the close time is reached.',
    authoritativeSource: 'Market close time recorded on-chain by the market contract.',
    terminal: false,
  },
  closed: {
    state: 'closed',
    label: 'Closed',
    description: 'The market is no longer accepting predictions and is awaiting resolution.',
    remainingActions: 'No new positions can be taken. You can review your position while awaiting resolution.',
    nextTransition: 'The market will move to "Resolving" when resolution begins.',
    authoritativeSource: 'Resolution schedule published by the market operator and enforced by the resolution contract.',
    terminal: false,
  },
  resolving: {
    state: 'resolving',
    label: 'Resolving',
    description: 'The market outcome is being determined from the resolution source.',
    remainingActions: 'No positions can be changed. You can monitor resolution progress.',
    nextTransition: 'The market will move to "Resolved" once the outcome is confirmed.',
    authoritativeSource: 'Resolution oracle / data source configured for the market.',
    terminal: false,
  },
  resolved: {
    state: 'resolved',
    label: 'Resolved',
    description: 'The market outcome has been confirmed and payouts can be determined.',
    remainingActions: 'Winning positions can be claimed once claiming opens.',
    nextTransition: 'The market will move to "Claimable" when payouts become available.',
    authoritativeSource: 'Resolution result recorded on-chain by the resolution contract.',
    terminal: false,
  },
  claimable: {
    state: 'claimable',
    label: 'Claimable',
    description: 'The market is resolved and payouts are available to claim.',
    remainingActions: 'You can claim your payout if you hold a winning position.',
    nextTransition: 'The market will move to "Claimed" once you claim your payout.',
    authoritativeSource: 'Payout calculation recorded on-chain by the market contract.',
    terminal: false,
  },
  claimed: {
    state: 'claimed',
    label: 'Claimed',
    description: 'Your payout for this market has been claimed.',
    remainingActions: 'No further actions are required for this market.',
    nextTransition: 'This is a terminal state for your position; no further transitions are expected.',
    authoritativeSource: 'Claim transaction recorded on-chain by the market contract.',
    terminal: true,
  },
  disputed: {
    state: 'disputed',
    label: 'Disputed',
    description: 'The market outcome has been disputed and is under review.',
    remainingActions: 'No positions can be changed while the dispute is reviewed.',
    nextTransition: 'The market will move to "Resolved" once the dispute is settled.',
    authoritativeSource: 'Dispute resolution process defined by the market governance contract.',
    terminal: false,
  },
  cancelled: {
    state: 'cancelled',
    label: 'Cancelled',
    description: 'The market has been cancelled and will not resolve to an outcome.',
    remainingActions: 'You can claim a refund of your position if applicable.',
    nextTransition: 'This is a terminal state; no further transitions are expected.',
    authoritativeSource: 'Cancellation recorded on-chain by the market contract.',
    terminal: true,
  },
};

const FALLBACK: LifecycleExplanation = {
  state: 'unknown',
  label: 'Unknown status',
  description: 'This market is in a status that is not yet recognized by this version of the app.',
  remainingActions: 'Available actions cannot be determined. Please refresh or update the app.',
  nextTransition: 'The next expected transition is unknown.',
  authoritativeSource: 'Refer to the market contract for the authoritative status.',
  terminal: false,
};

/**
 * Look up the reviewed explanation for a lifecycle state.
 * Unknown or future states return a safe fallback explanation.
 */
export function getLifecycleExplanation(state: string | null | undefined): LifecycleExplanation {
  if (!state) {
    return FALLBACK;
  }
  const key = state.toLowerCase() as LifecycleState;
  return CATALOG[key] ?? { ...FALLBACK, state };
}

/**
 * Build accessible announcement text for a status change.
 * Suitable for an aria-live region.
 */
export function getStatusAnnouncement(state: string | null | undefined): string {
  const explanation = getLifecycleExplanation(state);
  return `Market status changed to ${explanation.label}. ${explanation.description} ${explanation.remainingActions}`;
}

/**
 * All known lifecycle states, useful for tests and iteration.
 */
export const KNOWN_LIFECYCLE_STATES: LifecycleState[] = Object.keys(CATALOG) as LifecycleState[];
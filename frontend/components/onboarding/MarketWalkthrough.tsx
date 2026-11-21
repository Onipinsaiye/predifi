'use client';

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { persistentStorage } from '@/lib/persistentStorage';

const DISMISSAL_KEY = 'onboarding.marketWalkthrough.dismissed.v1';

interface WalkthroughStep {
  id: string;
  title: string;
  body: string;
  sample: {
    label: string;
    rows: Array<{ key: string; value: string }>;
  };
}

const STEPS: WalkthroughStep[] = [
  {
    id: 'close',
    title: 'How markets close',
    body:
      'Every market has a scheduled close time. When the close time is reached, trading stops and no new positions can be opened. Existing positions remain until the market is resolved. Close times are fixed when the market is created and cannot be changed by participants.',
    sample: {
      label: 'Sample market (non-executable)',
      rows: [
        { key: 'Question', value: 'Will the sample event happen by June 30?' },
        { key: 'Close time', value: 'June 30, 2025 · 23:59 UTC' },
        { key: 'Status', value: 'Open (sample data)' },
      ],
    },
  },
  {
    id: 'resolution',
    title: 'How markets resolve',
    body:
      'After a market closes, it is resolved against a defined source. A resolver records the outcome as Yes, No, or Invalid according to the resolution rules. Resolution does not happen automatically at close — it happens once the outcome is determined from the source.',
    sample: {
      label: 'Sample resolution (non-executable)',
      rows: [
        { key: 'Resolution source', value: 'Official sample data feed' },
        { key: 'Outcome', value: 'Yes (sample data)' },
        { key: 'Resolved at', value: 'July 1, 2025 · 12:00 UTC' },
      ],
    },
  },
  {
    id: 'claims',
    title: 'Claiming your outcome',
    body:
      'Once a market is resolved, holders of the winning outcome can claim their payout. Claiming is a separate action you take after resolution. If a market resolves as Invalid, positions are typically returned according to the market rules. You are never required to claim — unclaimed positions simply remain until you act.',
    sample: {
      label: 'Sample claim (non-executable)',
      rows: [
        { key: 'Your position', value: '10 Yes shares (sample data)' },
        { key: 'Outcome', value: 'Yes' },
        { key: 'Claimable', value: '10.00 (sample data)' },
      ],
    },
  },
  {
    id: 'fees',
    title: 'Fees you may see',
    body:
      'Markets may include fees such as a trading fee or a resolution fee. Fees are disclosed on each market before you participate. This walkthrough uses sample numbers only — always review the actual fee disclosure on a market before taking any action.',
    sample: {
      label: 'Sample fees (non-executable)',
      rows: [
        { key: 'Trading fee', value: '1.0% (sample data)' },
        { key: 'Resolution fee', value: '0.5% (sample data)' },
        { key: 'Total shown', value: '1.5% (sample data)' },
      ],
    },
  },
  {
    id: 'sources',
    title: 'Source rules',
    body:
      'Each market names the source used to determine its outcome and the rules for interpreting that source. If the source is unavailable or ambiguous, the market rules describe what happens next. Read the source and rules carefully — they define how a market resolves.',
    sample: {
      label: 'Sample source rules (non-executable)',
      rows: [
        { key: 'Primary source', value: 'Official sample data feed' },
        { key: 'Fallback', value: 'Secondary sample source' },
        { key: 'Ambiguity rule', value: 'Resolve as Invalid (sample data)' },
      ],
    },
  },
];

export interface MarketWalkthroughProps {
  /** Optional callback fired after the walkthrough is dismissed. */
  onDismiss?: () => void;
  /** Force the walkthrough to render regardless of stored dismissal (useful for tests/preview). */
  forceOpen?: boolean;
}

export function MarketWalkthrough({ onDismiss, forceOpen = false }: MarketWalkthroughProps) {
  const [isOpen, setIsOpen] = useState<boolean>(false);
  const [stepIndex, setStepIndex] = useState<number>(0);
  const [announcement, setAnnouncement] = useState<string>('');

  const dialogRef = useRef<HTMLDivElement | null>(null);
  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);

  const titleId = useId();
  const descriptionId = useId();
  const liveRegionId = useId();

  // Determine initial visibility from persistent storage (first visit only).
  useEffect(() => {
    if (forceOpen) {
      setIsOpen(true);
      return;
    }
    let dismissed = false;
    try {
      dismissed = persistentStorage.getItem(DISMISSAL_KEY) === 'true';
    } catch {
      dismissed = false;
    }
    setIsOpen(!dismissed);
  }, [forceOpen]);

  const totalSteps = STEPS.length;
  const currentStep = STEPS[stepIndex];

  const announce = useCallback((message: string) => {
    setAnnouncement(message);
  }, []);

  const persistDismissal = useCallback(() => {
    try {
      persistentStorage.setItem(DISMISSAL_KEY, 'true');
    } catch {
      // Storage may be unavailable; dismissal still applies for this session.
    }
  }, []);

  const handleDismiss = useCallback(() => {
    persistDismissal();
    setIsOpen(false);
    announce('Walkthrough dismissed.');
    onDismiss?.();
  }, [announce, onDismiss, persistDismissal]);

  const goToStep = useCallback(
    (nextIndex: number) => {
      const clamped = Math.max(0, Math.min(totalSteps - 1, nextIndex));
      setStepIndex(clamped);
      const step = STEPS[clamped];
      announce(`Step ${clamped + 1} of ${totalSteps}: ${step.title}`);
    },
    [announce, totalSteps],
  );

  const handleNext = useCallback(() => {
    if (stepIndex >= totalSteps - 1) {
      handleDismiss();
      return;
    }
    goToStep(stepIndex + 1);
  }, [goToStep, handleDismiss, stepIndex, totalSteps]);

  const handleBack = useCallback(() => {
    goToStep(stepIndex - 1);
  }, [goToStep, stepIndex]);

  // Focus management: move focus into the dialog on open, restore on close.
  useEffect(() => {
    if (!isOpen) return;
    previouslyFocusedRef.current = (document.activeElement as HTMLElement) ?? null;
    const raf = window.requestAnimationFrame(() => {
      headingRef.current?.focus();
    });
    return () => {
      window.cancelAnimationFrame(raf);
      const previous = previouslyFocusedRef.current;
      if (previous && typeof previous.focus === 'function') {
        previous.focus();
      }
    };
  }, [isOpen]);

  // Keyboard handling: Escape dismisses, arrows navigate, Tab is trapped within the dialog.
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        handleDismiss();
        return;
      }
      if (event.key === 'ArrowRight') {
        event.preventDefault();
        handleNext();
        return;
      }
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        handleBack();
        return;
      }
      if (event.key === 'Tab') {
        const dialog = dialogRef.current;
        if (!dialog) return;
        const focusable = dialog.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        const active = document.activeElement as HTMLElement | null;
        if (event.shiftKey) {
          if (active === first || !dialog.contains(active)) {
            event.preventDefault();
            last.focus();
          }
        } else if (active === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [handleBack, handleDismiss, handleNext, isOpen]);

  if (!isOpen) {
    return null;
  }

  const isFirstStep = stepIndex === 0;
  const isLastStep = stepIndex === totalSteps - 1;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      data-testid="market-walkthrough-overlay"
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        className="w-full max-w-lg rounded-lg bg-white p-6 shadow-xl"
        data-testid="market-walkthrough"
      >
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs font-medium uppercase tracking-wide text-gray-500">
            Step {stepIndex + 1} of {totalSteps}
          </span>
          <button
            type="button"
            onClick={handleDismiss}
            className="rounded px-2 py-1 text-sm text-gray-600 hover:bg-gray-100 focus:outline-none focus:ring-2 focus:ring-blue-500"
            aria-label="Dismiss walkthrough"
            data-testid="market-walkthrough-dismiss"
          >
            Dismiss
          </button>
        </div>

        <h2
          id={titleId}
          ref={headingRef}
          tabIndex={-1}
          className="text-lg font-semibold text-gray-900 focus:outline-none"
        >
          {currentStep.title}
        </h2>

        <p id={descriptionId} className="mt-2 text-sm text-gray-700">
          {currentStep.body}
        </p>

        <div className="mt-4 rounded border border-gray-200 bg-gray-50 p-3">
          <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500">
            {currentStep.sample.label}
          </p>
          <dl className="space-y-1">
            {currentStep.sample.rows.map((row) => (
              <div key={row.key} className="flex justify-between gap-4 text-sm">
                <dt className="text-gray-600">{row.key}</dt>
                <dd className="font-medium text-gray-900">{row.value}</dd>
              </div>
            ))}
          </dl>
        </div>

        <p className="mt-3 text-xs text-gray-500">
          This walkthrough uses sample data only. It does not connect a wallet or sign any
          transaction.
        </p>

        <div className="mt-6 flex items-center justify-between">
          <button
            type="button"
            onClick={handleBack}
            disabled={isFirstStep}
            className="rounded px-3 py-2 text-sm font-medium text-gray-700 hover:bg-gray-100 disabled:cursor-not-allowed disabled:opacity-40 focus:outline-none focus:ring-2 focus:ring-blue-500"
            data-testid="market-walkthrough-back"
          >
            Back
          </button>
          <button
            type="button"
            onClick={handleNext}
            className="rounded bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2"
            data-testid="market-walkthrough-next"
          >
            {isLastStep ? 'Finish' : 'Next'}
          </button>
        </div>

        <div
          id={liveRegionId}
          role="status"
          aria-live="polite"
          aria-atomic="true"
          className="sr-only"
          data-testid="market-walkthrough-live-region"
        >
          {announcement}
        </div>
      </div>
    </div>
  );
}

export default MarketWalkthrough;
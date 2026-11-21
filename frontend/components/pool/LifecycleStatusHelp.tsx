'use client';

import React, { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useAnnounce } from '@/hooks/useAnnounce';
import {
  getLifecycleStatusCopy,
  type LifecycleStatusCopy,
} from '@/lib/i18n/lifecycleStatusCatalog';

export interface LifecycleStatusHelpProps {
  /** Raw lifecycle status key coming from the API (may be an unknown future value). */
  status: string;
  /** Optional override for the visible label. */
  label?: string;
  /** Optional className applied to the wrapper element. */
  className?: string;
  /** When true, announces status changes via the live region. Defaults to true. */
  announceChanges?: boolean;
}

/**
 * Renders a market lifecycle status label with an accessible help popover
 * explaining what the state means, the next expected transition, and the
 * authoritative source for that information.
 *
 * All copy is sourced from the shared localization catalog so that
 * explanations stay consistent across the product.
 */
export function LifecycleStatusHelp({
  status,
  label,
  className,
  announceChanges = true,
}: LifecycleStatusHelpProps) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLSpanElement | null>(null);
  const popoverId = useId();
  const announce = useAnnounce();

  const copy: LifecycleStatusCopy = useMemo(
    () => getLifecycleStatusCopy(status),
    [status],
  );

  const isUnknown = copy.isFallback === true;
  const visibleLabel = label ?? copy.label;

  // Announce status changes to assistive technology.
  const previousStatusRef = useRef<string | null>(null);
  useEffect(() => {
    if (!announceChanges) {
      previousStatusRef.current = status;
      return;
    }
    if (previousStatusRef.current !== null && previousStatusRef.current !== status) {
      announce(copy.announcement);
    }
    previousStatusRef.current = status;
  }, [status, copy.announcement, announce, announceChanges]);

  // Close the popover on outside click and Escape.
  useEffect(() => {
    if (!open) return;

    const handlePointerDown = (event: MouseEvent | TouchEvent) => {
      if (
        wrapperRef.current &&
        event.target instanceof Node &&
        !wrapperRef.current.contains(event.target)
      ) {
        setOpen(false);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('touchstart', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('touchstart', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  const toggle = () => setOpen((prev) => !prev);

  return (
    <span
      ref={wrapperRef}
      className={['lifecycle-status-help', className].filter(Boolean).join(' ')}
      data-status={status}
      data-unknown={isUnknown ? 'true' : undefined}
    >
      <span className="lifecycle-status-help__label">{visibleLabel}</span>
      <button
        type="button"
        className="lifecycle-status-help__trigger"
        aria-label={copy.triggerLabel}
        aria-expanded={open}
        aria-controls={popoverId}
        aria-haspopup="dialog"
        onClick={toggle}
        onMouseEnter={() => setOpen(true)}
        onFocus={() => setOpen(true)}
      >
        <span aria-hidden="true">?</span>
      </button>

      {open && (
        <span
          id={popoverId}
          role="dialog"
          aria-label={copy.popoverTitle}
          className="lifecycle-status-help__popover"
        >
          <span className="lifecycle-status-help__title">{copy.popoverTitle}</span>
          <span className="lifecycle-status-help__section">
            <span className="lifecycle-status-help__section-heading">
              {copy.explanationHeading}
            </span>
            <span className="lifecycle-status-help__section-body">
              {copy.explanation}
            </span>
          </span>
          <span className="lifecycle-status-help__section">
            <span className="lifecycle-status-help__section-heading">
              {copy.nextTransitionHeading}
            </span>
            <span className="lifecycle-status-help__section-body">
              {copy.nextTransition}
            </span>
          </span>
          <span className="lifecycle-status-help__section">
            <span className="lifecycle-status-help__section-heading">
              {copy.sourceHeading}
            </span>
            <span className="lifecycle-status-help__section-body">
              {copy.source}
            </span>
          </span>
        </span>
      )}
    </span>
  );
}

export default LifecycleStatusHelp;
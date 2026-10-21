'use client';

import { useEffect, useMemo, useState } from 'react';
import Link from 'next/link';
import { usePathname, useRouter } from 'next/navigation';

export interface RouteErrorFallbackProps {
  error: Error & { digest?: string };
  reset: () => void;
  routeLabel?: string;
  retryLabel?: string;
}

export type RouteErrorKind = 'not-found' | 'transient' | 'unknown';

function classifyError(error: Error): RouteErrorKind {
  const name = error.name?.toLowerCase() ?? '';
  const message = error.message?.toLowerCase() ?? '';

  if (
    name === 'notfounderror' ||
    name === 'not-found' ||
    message.includes('404') ||
    message.includes('not found')
  ) {
    return 'not-found';
  }

  if (
    name === 'typeerror' ||
    name === 'networkerror' ||
    name === 'timeouterror' ||
    message.includes('failed to fetch') ||
    message.includes('network request failed') ||
    message.includes('503')
  ) {
    return 'transient';
  }

  return 'unknown';
}

function redactDiagnostics(error: Error & { digest?: string }): { name: string; digest: string | null } {
  const name = typeof error.name === 'string' && error.name.length > 0 ? error.name : 'Error';
  const digest = typeof error.digest === 'string' && error.digest.length > 0 ? error.digest : null;
  return { name, digest };
}

export default function RouteErrorFallback({
  error,
  reset,
  routeLabel = 'this page',
  retryLabel = 'Try again',
}: RouteErrorFallbackProps) {
  const router = useRouter();
  const pathname = usePathname();
  const [retrying, setRetrying] = useState(false);
  const kind = useMemo(() => classifyError(error), [error]);
  const diagnostics = useMemo(() => redactDiagnostics(error), [error]);

  useEffect(() => {
    setRetrying(false);
  }, [error]);

  const handleRetry = () => {
    setRetrying(true);
    reset();
  };

  const handleRefresh = () => {
    setRetrying(true);
    router.refresh();
  };

  const isNotFound = kind === 'not-found';

  return (
    <section
      role="alert"
      aria-live="assertive"
      data-route-error-kind={kind}
      className="mx-auto flex max-w-2xl flex-col gap-4 p-6"
    >
      <h1 className="text-xl font-semibold">
        {isNotFound ? `${routeLabel} could not be found` : `Failed to load ${routeLabel}`}
      </h1>
      <p className="text-sm text-gray-700">
        {isNotFound
          ? 'The requested market resource does not exist or is no longer available.'
          : 'A temporary problem prevented this view from loading. Your current view and filters are preserved.'}
      </p>
      <dl className="grid grid-cols-2 gap-1 text-xs text-gray-600">
        <dt>Route</dt>
        <dd><span className="font-mono">{pathname || '/'}</span></dd>
        <dt>Error type</dt>
        <dd>{diagnostics.name}</dd>
        {diagnostics.digest ? (
          <>
            <dt>Diagnostic ID</dt>
            <dd><span className="font-mono">{diagnostics.digest}</span></dd>
          </>
        ) : null}
      </dl>
      <div className="flex flex-wrap gap-2">
        {!diagnostics.digest && !isNotFound ? (
          <button
            type="button"
            onClick={handleRetry}
            disabled={retrying}
            className="rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
          >
            {retrying ? 'Retrying…' : retryLabel}
          </button>
        ) : null}
        <button
          type="button"
          onClick={handleRefresh}
          disabled={retrying}
          className="rounded border border-gray-300 px-4 py-2 text-sm font-medium disabled:opacity-50"
        >
          {retrying ? 'Refreshing…' : 'Refresh route'}
        </button>
        <Link
          href="/dashboard"
          className="rounded border border-gray-300 px-4 py-2 text-sm font-medium"
        >
          Back to dashboard
        </Link>
      </div>
    </section>
  );
}
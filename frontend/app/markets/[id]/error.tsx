'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';

type MarketErrorProps = {
  error: Error & { digest?: string };
  reset: () => void;
};

function isNotFoundError(error: Error): boolean {
  const message = error.message.toLowerCase();
  return (
    error.name === 'NotFoundError' ||
    message.includes('not found') ||
    message.includes('404')
  );
}

function redact(error: Error & { digest?: string }) {
  return {
    name: error.name,
    digest: error.digest ?? null,
  };
}

export default function MarketError({ error, reset }: MarketErrorProps) {
  const router = useRouter();
  const notFound = isNotFoundError(error);
  const diagnostics = redact(error);

  useEffect(() => {
    console.error('[markets/id] boundary caught error', diagnostics);
  }, [diagnostics]);

  if (notFound) {
    return (
      <section role="alert" aria-live="polite">
        <h2>Market not found</h2>
        <p>The market you requested does not exist or is no longer available.</p>
        <button type="button" onClick={() => router.push('/markets')}>
          Back to markets
        </button>
      </section>
    );
  }

  return (
    <section role="alert" aria-live="assertive">
      <h2>Something went wrong</h2>
      <p>We couldn&#39;t load this market right now. Please retry.</p>
      <p>
        <small>
          Reference: ?{diagnostics.digest ?? diagnostics.name}
        </small>
      </p>
      <button type="button" onClick={() => reset()}>
        Retry
      </button>
      <button type="button" onClick={() => router.push('/markets')}>
        Back to markets
      </button>
    </section>
  );
}

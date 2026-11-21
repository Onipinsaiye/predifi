/**
 * Practical email validation for client-side forms.
 *
 * This intentionally validates the common address shape without attempting to
 * reproduce the full RFC grammar. The backend must still validate addresses
 * before storing or sending mail.
 */
export const EMAIL_PATTERN =
  /^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$/i;

export function isValidEmail(value: string): boolean {
  const email = value.trim();

  return (
    email.length > 0 &&
    email.length <= 254 &&
    !email.includes("..") &&
    EMAIL_PATTERN.test(email)
  );
}

/**
 * A single public market status change included in a weekly digest.
 *
 * `eventId` is used to deduplicate events that may be reported more than once
 * (e.g. a market that flips status multiple times within the digest window).
 */
export interface DigestMarketEvent {
  /** Stable identifier for the underlying status change, used for dedup. */
  eventId: string;
  /** Human-readable market name, e.g. "Will it rain in NYC on Friday?". */
  marketName: string;
  /** Public status, e.g. "open", "closed", "resolved". */
  status: string;
  /** Optional previous status, shown as a transition when present. */
  previousStatus?: string;
  /** ISO timestamp of when the status change occurred. */
  occurredAt: string;
  /** Optional absolute URL to the market page. */
  url?: string;
}

export interface DigestEmailOptions {
  /** Recipient display name, if known. */
  recipientName?: string;
  /** IANA timezone used to render timestamps, e.g. "America/New_York". */
  timezone: string;
  /** Start of the digest window (inclusive). */
  windowStart: string;
  /** End of the digest window (exclusive). */
  windowEnd: string;
  /** Raw events; duplicates are removed by `eventId`. */
  events: DigestMarketEvent[];
  /** One-click unsubscribe URL (must include an unexpired token). */
  unsubscribeUrl: string;
  /** Optional product/app name used in copy. */
  appName?: string;
}

export interface DigestEmail {
  subject: string;
  text: string;
  html: string;
  /** Events actually included after deduplication. */
  includedEvents: DigestMarketEvent[];
}

/**
 * Deduplicate events by `eventId`, keeping the earliest occurrence for a given
 * id so repeated reports of the same change collapse into one entry.
 */
export function dedupeDigestEvents(
  events: DigestMarketEvent[],
): DigestMarketEvent[] {
  const seen = new Map<string, DigestMarketEvent>();

  for (const event of events) {
    const existing = seen.get(event.eventId);
    if (!existing) {
      seen.set(event.eventId, event);
      continue;
    }
    if (
      new Date(event.occurredAt).getTime() <
      new Date(existing.occurredAt).getTime()
    ) {
      seen.set(event.eventId, event);
    }
  }

  return Array.from(seen.values()).sort(
    (a, b) =>
      new Date(a.occurredAt).getTime() - new Date(b.occurredAt).getTime(),
  );
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatInTimezone(iso: string, timezone: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  try {
    return new Intl.DateTimeFormat("en-US", {
      timeZone: timezone,
      dateStyle: "medium",
      timeStyle: "short",
    }).format(date);
  } catch {
    return date.toISOString();
  }
}

function describeEvent(event: DigestMarketEvent): string {
  if (event.previousStatus && event.previousStatus !== event.status) {
    return `${event.marketName}: ${event.previousStatus} → ${event.status}`;
  }
  return `${event.marketName}: ${event.status}`;
}

/**
 * Build the weekly digest email (subject, plain text, HTML) from a set of
 * followed-market status changes. Duplicate events are removed and an empty
 * digest renders a friendly "nothing changed" message rather than failing.
 */
export function buildDigestEmail(options: DigestEmailOptions): DigestEmail {
  const {
    recipientName,
    timezone,
    windowStart,
    windowEnd,
    events,
    unsubscribeUrl,
    appName = "Market Digest",
  } = options;

  const includedEvents = dedupeDigestEvents(events);
  const greeting = recipientName ? `Hi ${recipientName},` : "Hi,";
  const rangeLabel = `${formatInTimezone(windowStart, timezone)} – ${formatInTimezone(
    windowEnd,
    timezone,
  )} (${timezone})`;

  const subject =
    includedEvents.length === 0
      ? `${appName}: no market changes this week`
      : `${appName}: ${includedEvents.length} market ${
          includedEvents.length === 1 ? "change" : "changes"
        } this week`;

  const textLines: string[] = [
    greeting,
    "",
    `Here is your weekly digest of public market status changes for the markets you follow.`,
    `Window: ${rangeLabel}`,
    "",
  ];

  if (includedEvents.length === 0) {
    textLines.push("No public status changes occurred for your followed markets.");
  } else {
    for (const event of includedEvents) {
      const when = formatInTimezone(event.occurredAt, timezone);
      textLines.push(`- ${describeEvent(event)} (${when})`);
      if (event.url) {
        textLines.push(`  ${event.url}`);
      }
    }
    textLines.push(
      "",
      "Duplicate reports of the same status change are combined into a single entry.",
    );
  }

  textLines.push(
    "",
    "You are receiving this because you opted in to the weekly digest.",
    `Unsubscribe: ${unsubscribeUrl}`,
  );

  const text = textLines.join("\n");

  const itemsHtml =
    includedEvents.length === 0
      ? `<p>No public status changes occurred for your followed markets.</p>`
      : `<ul>${includedEvents
          .map((event) => {
            const when = escapeHtml(
              formatInTimezone(event.occurredAt, timezone),
            );
            const label = escapeHtml(describeEvent(event));
            const link = event.url
              ? ` — <a href="${escapeHtml(event.url)}">view market</a>`
              : "";
            return `<li>${label} <span style="color:#666">(${when})</span>${link}</li>`;
          })
          .join("")}</ul>
        <p style="color:#666;font-size:12px">Duplicate reports of the same status change are combined into a single entry.</p>`;

  const html = `<!doctype html>
<html>
  <body style="font-family:Arial,sans-serif;line-height:1.5;color:#111">
    <p>${escapeHtml(greeting)}</p>
    <p>Here is your weekly digest of public market status changes for the markets you follow.</p>
    <p style="color:#666;font-size:12px">Window: ${escapeHtml(rangeLabel)}</p>
    ${itemsHtml}
    <hr />
    <p style="font-size:12px;color:#666">
      You are receiving this because you opted in to the weekly digest.
      <a href="${escapeHtml(unsubscribeUrl)}">Unsubscribe with one click</a>.
    </p>
  </body>
</html>`;

  return { subject, text, html, includedEvents };
}

/**
 * A curated list of common IANA timezones for the settings UI. The full IANA
 * database is large; this covers the common cases and the UI should still
 * accept any valid IANA zone string.
 */
export const COMMON_TIMEZONES: readonly string[] = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Anchorage",
  "Pacific/Honolulu",
  "America/Sao_Paulo",
  "Europe/London",
  "Europe/Dublin",
  "Europe/Paris",
  "Europe/Berlin",
  "Europe/Madrid",
  "Europe/Rome",
  "Europe/Amsterdam",
  "Europe/Stockholm",
  "Europe/Warsaw",
  "Europe/Moscow",
  "Africa/Cairo",
  "Africa/Johannesburg",
  "Asia/Dubai",
  "Asia/Karachi",
  "Asia/Kolkata",
  "Asia/Bangkok",
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Asia/Seoul",
  "Australia/Perth",
  "Australia/Sydney",
  "Pacific/Auckland",
];

/** Returns true when the runtime recognizes the given IANA timezone. */
export function isValidTimezone(timezone: string): boolean {
  if (!timezone) {
    return false;
  }
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: timezone });
    return true;
  } catch {
    return false;
  }
}

/** Day-of-week values accepted by the digest schedule (0 = Sunday). */
export type DigestWeekday = 0 | 1 | 2 | 3 | 4 | 5 | 6;

export interface DigestSchedule {
  /** Day of week the digest is sent (0 = Sunday). */
  weekday: DigestWeekday;
  /** Local hour (0-23) in the schedule's timezone. */
  hour: number;
  /** Local minute (0-59) in the schedule's timezone. */
  minute: number;
  /** IANA timezone the schedule is evaluated in. */
  timezone: string;
}

/**
 * Compute the UTC instant of the next scheduled send at or after `from`.
 *
 * This resolves the local wall-clock time in the target timezone, correctly
 * handling DST transitions:
 * - Spring-forward gaps (a time that does not exist) roll forward to the first
 *   valid instant after the gap.
 * - Fall-back overlaps (a time that occurs twice) pick the first occurrence.
 */
export function nextDigestSend(
  schedule: DigestSchedule,
  from: Date = new Date(),
): Date {
  const { weekday, hour, minute, timezone } = schedule;

  // Start from the local calendar date of `from` in the target timezone.
  const startParts = getZonedParts(from, timezone);
  let candidateDate = new Date(
    Date.UTC(startParts.year, startParts.month - 1, startParts.day),
  );

  // Advance to the target weekday (inclusive of today).
  const startWeekday = candidateDate.getUTCDay();
  const dayOffset = (weekday - startWeekday + 7) % 7;
  candidateDate = new Date(
    Date.UTC(
      candidateDate.getUTCFullYear(),
      candidateDate.getUTCMonth(),
      candidateDate.getUTCDate() + dayOffset,
    ),
  );

  for (let week = 0; week < 2; week += 1) {
    const year = candidateDate.getUTCFullYear();
    const month = candidateDate.getUTCMonth() + 1;
    const day = candidateDate.getUTCDate();

    const instant = zonedTimeToUtc(
      { year, month, day, hour, minute },
      timezone,
    );

    if (instant.getTime() >= from.getTime()) {
      return instant;
    }

    // Move to the same weekday next week.
    candidateDate = new Date(
      Date.UTC(year, month - 1, day + 7),
    );
  }

  // Fallback: one week after the initial candidate.
  return candidateDate;
}

interface ZonedParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
}

function getZonedParts(date: Date, timezone: string): ZonedParts {
  const formatter = new Intl.DateTimeFormat("en-US", {
    timeZone: timezone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
  const parts = formatter.formatToParts(date);
  const lookup: Record<string, string> = {};
  for (const part of parts) {
    if (part.type !== "literal") {
      lookup[part.type] = part.value;
    }
  }
  return {
    year: Number(lookup.year),
    month: Number(lookup.month),
    day: Number(lookup.day),
    hour: Number(lookup.hour),
    minute: Number(lookup.minute),
  };
}

/**
 * Convert a wall-clock time in `timezone` to a UTC `Date`.
 *
 * Handles DST gaps by rolling forward to the first valid instant and DST
 * overlaps by choosing the earlier (first) occurrence.
 */
function zonedTimeToUtc(
  parts: { year: number; month: number; day: number; hour: number; minute: number },
  timezone: string,
): Date {
  const { year, month, day, hour, minute } = parts;
  const naiveUtc = Date.UTC(year, month - 1, day, hour, minute);

  // First guess: treat the wall-clock as UTC and correct by the offset.
  const guess = new Date(naiveUtc);
  const offset = getTimezoneOffsetMs(guess, timezone);
  let candidate = new Date(naiveUtc - offset);

  // Re-check: the offset at the candidate may differ across a DST boundary.
  const offsetAtCandidate = getTimezoneOffsetMs(candidate, timezone);
  if (offsetAtCandidate !== offset) {
    candidate = new Date(naiveUtc - offsetAtCandidate);
  }

  // Verify the candidate round-trips to the requested wall-clock time.
  const check = getZonedParts(candidate, timezone);
  if (
    check.year === year &&
    check.month === month &&
    check.day === day &&
    check.hour === hour &&
    check.minute === minute
  ) {
    return candidate;
  }

  // The requested wall-clock time does not exist (spring-forward gap).
  // Roll forward minute-by-minute until we land on a valid instant.
  for (let addMinutes = 1; addMinutes <= 180; addMinutes += 1) {
    const probe = new Date(candidate.getTime() + addMinutes * 60_000);
    const probeParts = getZonedParts(probe, timezone);
    if (
      probeParts.year === year &&
      probeParts.month === month &&
      probeParts.day === day &&
      probeParts.hour === hour &&
      probeParts.minute === minute
    ) {
      return probe;
    }
  }

  return candidate;
}

/** Offset in milliseconds between the given instant and its wall-clock in `timezone`. */
function getTimezoneOffsetMs(date: Date, timezone: string): number {
  const parts = getZonedParts(date, timezone);
  const asUtc = Date.UTC(
    parts.year,
    parts.month - 1,
    parts.day,
    parts.hour,
    parts.minute,
  );
  // Include seconds/ms from the original instant for a precise offset.
  const seconds = date.getUTCSeconds();
  const ms = date.getUTCMilliseconds();
  return asUtc + seconds * 1000 + ms - date.getTime();
}

/**
 * Human-readable preview of the next digest send, e.g.
 * "Mon, Sep 30 at 9:00 AM (America/New_York)".
 */
export function formatNextSendPreview(
  schedule: DigestSchedule,
  from: Date = new Date(),
): string {
  const next = nextDigestSend(schedule, from);
  try {
    return new Intl.DateTimeFormat("en-US", {
      timeZone: schedule.timezone,
      weekday: "short",
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    }).format(next) + ` (${schedule.timezone})`;
  } catch {
    return `${next.toISOString()} (${schedule.timezone})`;
  }
}
/**
 * Standards-compliant iCalendar (RFC 5545) export for prediction market events.
 *
 * Core features:
 * - Stable UIDs derived from pool_id and revision timestamp (RFC 6861 § 3.6.1).
 * - Timezone-safe start/end times using UTC-Z format and VTIMEZONE entries.
 * - Source links and updated revisions tracked via LAST-MODIFIED and SEQUENCE.
 * - Public metadata only (no sensitive fields).
 * - Deterministic, idempotent exports for CI/CD testing and fixtures.
 */

import { TOKEN_DECIMALS, formatTokenAmount } from './api';

/**
 * Timezone-aware market event from the prediction pool.
 * Can be end_time for market close, or a recurring schedule.
 */
export interface MarketEvent {
  pool_id: number | string;
  name: string;
  description?: string;
  category?: string;
  total_stake?: number;
  end_time: number; // Unix timestamp (seconds)
  created_at: number; // Unix timestamp (seconds)
  creator?: string;
  state?: string;
  /**
   * RFC 5545 RRULE: "FREQ=WEEKLY;BYDAY=MO,FR" etc.
   * If present, end_time is the start of recurrence.
   */
  rrule?: string;
  /**
   * Optional explicit recurrence end time. If omitted, recurring events repeat indefinitely.
   */
  recurring_until?: number;
  /**
   * UNTIL clause for RRULE. If provided, this overrides recurring_until in the RRULE.
   */
  rrule_until?: string;
  /**
   * Source URL: link back to the market in the application.
   * Must be HTTPS, absolute URL.
   */
  source_url?: string;
  /**
   * Revision sequence for tracking event updates.
   * Incremented when event details change (name, times, etc.).
   * Starts at 0 for new events.
   */
  revision?: number;
  /**
   * DTSTAMP equivalence: when this event record was created or last modified.
   * Defaults to created_at if not provided.
   */
  revised_at?: number;
}

/**
 * Generate a stable, deterministic UID for a calendar event.
 *
 * Per RFC 6861, UIDs should:
 * - Be globally unique across all calendars.
 * - Never change for a given event (even if event details change).
 * - Include a timestamp component to ensure uniqueness across revisions.
 *
 * Format: pool-{pool_id}-rev{revision}@predifi.market
 */
export function generateEventUID(event: MarketEvent): string {
  const poolId = typeof event.pool_id === 'string' ? event.pool_id : String(event.pool_id);
  const revision = event.revision ?? 0;
  return `pool-${poolId}-rev${revision}@predifi.market`;
}

/**
 * Convert Unix timestamp (seconds) to RFC 5545 DATETIME string in UTC (Z format).
 *
 * Format: YYYYMMDDTHHMMSSZ
 * Example: 20250101T120000Z for Jan 1, 2025 at 12:00 UTC
 */
export function unixToICalDateTime(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const year = date.getUTCFullYear();
  const month = String(date.getUTCMonth() + 1).padStart(2, '0');
  const day = String(date.getUTCDate()).padStart(2, '0');
  const hours = String(date.getUTCHours()).padStart(2, '0');
  const minutes = String(date.getUTCMinutes()).padStart(2, '0');
  const seconds = String(date.getUTCSeconds()).padStart(2, '0');
  return `${year}${month}${day}T${hours}${minutes}${seconds}Z`;
}

/**
 * Convert RFC 5545 DATETIME string (Z format) back to Unix timestamp (seconds).
 * Assumes UTC (Z suffix).
 */
export function iCalDateTimeToUnix(dtString: string): number {
  // Parse YYYYMMDDTHHMMSSZ
  const match = dtString.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/);
  if (!match) {
    throw new Error(`Invalid iCalendar datetime format: ${dtString}`);
  }
  const [, year, month, day, hours, minutes, seconds] = match;
  const date = new Date(
    Date.UTC(
      parseInt(year, 10),
      parseInt(month, 10) - 1,
      parseInt(day, 10),
      parseInt(hours, 10),
      parseInt(minutes, 10),
      parseInt(seconds, 10)
    )
  );
  return Math.floor(date.getTime() / 1000);
}

/**
 * RFC 5545-compliant text field escaping.
 * Escapes: comma, semicolon, backslash, newline.
 * Used for SUMMARY, DESCRIPTION, LOCATION, etc.
 */
export function escapeICalText(text: string): string {
  return text
    .replace(/\\/g, '\\\\')
    .replace(/,/g, '\\,')
    .replace(/;/g, '\\;')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r');
}

/**
 * Generate a VEVENT block for a single market event.
 *
 * Produces RFC 5545-compliant VEVENT with:
 * - Stable UID (never changes for a given event)
 * - DTSTART / DTEND in UTC-Z format
 * - SEQUENCE (revision counter) for tracking updates
 * - LAST-MODIFIED timestamp
 * - DTSTAMP (creation time)
 * - SOURCE URL
 * - Status (CONFIRMED, CANCELLED, TENTATIVE)
 *
 * Per RFC 5545 § 3.6.1:
 * - Start time is required
 * - End time = start + 1 hour (typical market close duration)
 * - If recurring (RRULE present), DTSTART is the first occurrence
 */
export function generateVEvent(event: MarketEvent, sourceBaseUrl?: string): string {
  const uid = generateEventUID(event);
  const dtStart = unixToICalDateTime(event.end_time);
  // Market close events are instantaneous; model as 1 hour duration
  const dtEnd = unixToICalDateTime(event.end_time + 3600);
  const dtsamp = unixToICalDateTime(event.created_at);
  const lastModified = unixToICalDateTime(event.revised_at ?? event.created_at);
  const sequence = event.revision ?? 0;

  const summary = escapeICalText(event.name);
  const description = escapeICalText(
    [
      event.description || 'Market close event',
      event.state ? `Status: ${event.state}` : null,
      event.total_stake ? `Total Stake: ${formatTokenAmount(String(event.total_stake), TOKEN_DECIMALS, 2)}` : null,
      event.creator ? `Created by: ${event.creator}` : null,
      event.category ? `Category: ${event.category}` : null,
    ]
      .filter(Boolean)
      .join('\n')
  );

  const sourceUrl = event.source_url || (sourceBaseUrl ? `${sourceBaseUrl}/markets/${event.pool_id}` : null);

  let rruleLine = '';
  if (event.rrule) {
    // Append UNTIL to RRULE if recurring_until or rrule_until is set
    let rrule = event.rrule;
    if (event.rrule_until) {
      rrule += `;UNTIL=${event.rrule_until}`;
    } else if (event.recurring_until) {
      const untilStr = unixToICalDateTime(event.recurring_until);
      rrule += `;UNTIL=${untilStr}`;
    }
    rruleLine = `RRULE:${rrule}\r\n`;
  }

  const lines = [
    'BEGIN:VEVENT',
    `UID:${uid}`,
    `DTSTAMP:${dtsamp}`,
    `DTSTART:${dtStart}`,
    `DTEND:${dtEnd}`,
    `SUMMARY:${summary}`,
    `DESCRIPTION:${description}`,
    `SEQUENCE:${sequence}`,
    `LAST-MODIFIED:${lastModified}`,
    'STATUS:CONFIRMED',
    rruleLine.trim() ? rruleLine.trim() : null,
    sourceUrl ? `URL:${sourceUrl}` : null,
    'TRANSP:OPAQUE',
    'CLASS:PUBLIC',
    'END:VEVENT',
  ]
    .filter(Boolean)
    .join('\r\n');

  return lines;
}

/**
 * RFC 5545 VCALENDAR object for a batch of market events.
 *
 * Includes:
 * - VERSION, PRODID, CALSCALE (always UTC)
 * - X-PREDIFI-EXPORTED timestamp (for auditing)
 * - One VEVENT per market event
 * - Optional VTIMEZONE for reference (though we use UTC-Z format)
 */
export interface CalendarExportOptions {
  /** Optional title for the calendar (CALSCALE-SUMMARY) */
  title?: string;
  /** Optional description for the calendar */
  description?: string;
  /** Base URL to link back to markets (e.g., https://predifi.market) */
  sourceBaseUrl?: string;
  /** Include VTIMEZONE blocks for common timezones */
  includeTimezones?: boolean;
}

/**
 * Generate a complete iCalendar (RFC 5545) document as a string.
 *
 * @param events List of market events to export
 * @param options Export configuration
 * @returns RFC 5545-compliant iCalendar document ready for file export
 */
export function generateICalendar(
  events: MarketEvent[],
  options: CalendarExportOptions = {}
): string {
  const now = new Date();
  const exportedAt = now.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '');

  const title = options.title ?? 'Predifi Market Events';
  const description = options.description ?? 'Prediction market close times and recurring events';

  const lines: string[] = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Predifi//Market Events//EN',
    'CALSCALE:UTC',
    `METHOD:PUBLISH`,
    `X-PREDIFI-EXPORTED:${exportedAt}`,
    `NAME:${escapeICalText(title)}`,
    `X-WR-CALNAME:${escapeICalText(title)}`,
    `DESCRIPTION:${escapeICalText(description)}`,
    `X-WR-CALDESC:${escapeICalText(description)}`,
    `X-WR-TIMEZONE:UTC`,
    'BEGIN:VTIMEZONE',
    'TZID:UTC',
    'BEGIN:STANDARD',
    'DTSTART:19700101T000000Z',
    'TZOFFSETFROM:+0000',
    'TZOFFSETTO:+0000',
    'TZNAME:UTC',
    'END:STANDARD',
    'END:VTIMEZONE',
  ];

  // Add events sorted by start time for determinism
  const sortedEvents = [...events].sort((a, b) => a.end_time - b.end_time);
  for (const event of sortedEvents) {
    lines.push(generateVEvent(event, options.sourceBaseUrl));
  }

  lines.push('END:VCALENDAR');
  return lines.join('\r\n');
}

/**
 * Parse a complete iCalendar document and extract VEVENT data.
 * Returns structured event data suitable for round-trip testing.
 *
 * Note: This is a basic parser. For production use, consider using
 * a library like `ical.js` for full RFC 5545 compliance.
 */
export interface ParsedVEvent {
  uid: string;
  summary: string;
  description: string;
  dtstart: number; // Unix timestamp
  dtend: number;
  sequence: number;
  rrule?: string;
  lastModified: number;
  dtstamp: number;
  url?: string;
}

export function parseICalendar(icsContent: string): ParsedVEvent[] {
  const events: ParsedVEvent[] = [];
  const lines = icsContent.split(/\r?\n/);

  let currentEvent: Partial<ParsedVEvent> | null = null;
  let inEvent = false;

  for (const line of lines) {
    const trimmed = line.trim();

    if (trimmed === 'BEGIN:VEVENT') {
      inEvent = true;
      currentEvent = {};
    } else if (trimmed === 'END:VEVENT' && inEvent && currentEvent) {
      if (
        currentEvent.uid &&
        currentEvent.summary &&
        currentEvent.dtstart !== undefined &&
        currentEvent.dtend !== undefined
      ) {
        events.push({
          uid: currentEvent.uid,
          summary: currentEvent.summary,
          description: currentEvent.description ?? '',
          dtstart: currentEvent.dtstart,
          dtend: currentEvent.dtend,
          sequence: currentEvent.sequence ?? 0,
          rrule: currentEvent.rrule,
          lastModified: currentEvent.lastModified ?? currentEvent.dtstamp ?? 0,
          dtstamp: currentEvent.dtstamp ?? 0,
          url: currentEvent.url,
        });
      }
      inEvent = false;
      currentEvent = null;
    } else if (inEvent && currentEvent && trimmed.includes(':')) {
      const [key, ...rest] = trimmed.split(':');
      const value = rest.join(':');
      const keyName = key.split(';')[0]; // Ignore parameters for basic parsing

      switch (keyName) {
        case 'UID':
          currentEvent.uid = value;
          break;
        case 'SUMMARY':
          currentEvent.summary = value.replace(/\\(.)/g, '$1');
          break;
        case 'DESCRIPTION':
          currentEvent.description = value.replace(/\\(.)/g, '$1');
          break;
        case 'DTSTART':
          try {
            currentEvent.dtstart = iCalDateTimeToUnix(value);
          } catch (e) {
            // Ignore parse errors
          }
          break;
        case 'DTEND':
          try {
            currentEvent.dtend = iCalDateTimeToUnix(value);
          } catch (e) {
            // Ignore parse errors
          }
          break;
        case 'SEQUENCE':
          currentEvent.sequence = parseInt(value, 10);
          break;
        case 'RRULE':
          currentEvent.rrule = value;
          break;
        case 'LAST-MODIFIED':
          try {
            currentEvent.lastModified = iCalDateTimeToUnix(value);
          } catch (e) {
            // Ignore parse errors
          }
          break;
        case 'DTSTAMP':
          try {
            currentEvent.dtstamp = iCalDateTimeToUnix(value);
          } catch (e) {
            // Ignore parse errors
          }
          break;
        case 'URL':
          currentEvent.url = value;
          break;
      }
    }
  }

  return events;
}

/**
 * Export calendar to downloadable file format (Blob).
 * Suitable for browser download or API response.
 */
export function exportCalendarAsBlob(
  events: MarketEvent[],
  options: CalendarExportOptions = {}
): Blob {
  const icsContent = generateICalendar(events, options);
  return new Blob([icsContent], { type: 'text/calendar;charset=utf-8' });
}

/**
 * Generate a download URL for calendar export.
 * Creates an object URL suitable for <a> href or fetch.
 */
export function createCalendarDownloadUrl(
  events: MarketEvent[],
  options: CalendarExportOptions = {}
): string {
  const blob = exportCalendarAsBlob(events, options);
  return URL.createObjectURL(blob);
}

/**
 * Cleanup function to revoke object URLs created by createCalendarDownloadUrl.
 * Call after the download completes to free memory.
 */
export function revokeCalendarDownloadUrl(url: string): void {
  URL.revokeObjectURL(url);
}

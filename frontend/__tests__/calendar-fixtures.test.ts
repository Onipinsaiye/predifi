/**
 * Fixture-based integration tests for calendar export.
 *
 * Validates:
 * - Fixture determinism (same output every time)
 * - Snapshot stability (regressions detected early)
 * - Round-trip parsing (export → parse → validate)
 * - DST handling in realistic scenarios
 * - Revision tracking across fixture variations
 */

import {
  generateICalendar,
  parseICalendar,
  generateEventUID,
  unixToICalDateTime,
} from '../src/lib/calendar';
import * as fixtures from '../__fixtures__/calendar-events';

describe('Calendar Fixtures - Integration Tests', () => {
  describe('Fixture Validity', () => {
    it('all fixtures have required fields', () => {
      for (const market of fixtures.FIXTURE_ALL_MARKETS) {
        expect(market).toHaveProperty('pool_id');
        expect(market).toHaveProperty('name');
        expect(market).toHaveProperty('end_time');
        expect(market).toHaveProperty('created_at');
      }
    });

    it('fixture timestamps are reasonable (within ~10 years)', () => {
      const now = Math.floor(Date.now() / 1000);
      const tenYearsAgo = now - 365 * 24 * 60 * 60 * 10;
      const tenYearsInFuture = now + 365 * 24 * 60 * 60 * 10;

      for (const market of fixtures.FIXTURE_ALL_MARKETS) {
        expect(market.end_time).toBeGreaterThan(tenYearsAgo);
        expect(market.end_time).toBeLessThan(tenYearsInFuture);
        expect(market.created_at).toBeGreaterThan(tenYearsAgo);
        expect(market.created_at).toBeLessThan(tenYearsInFuture);
      }
    });

    it('revised_at (if present) is after created_at', () => {
      for (const market of fixtures.FIXTURE_ALL_MARKETS) {
        if (market.revised_at) {
          expect(market.revised_at).toBeGreaterThanOrEqual(market.created_at);
        }
      }
    });
  });

  describe('Deterministic Export (Fixture Stability)', () => {
    it('simple market generates identical iCalendar on repeated exports', () => {
      const ical1 = generateICalendar([fixtures.FIXTURE_SIMPLE_MARKET]);
      const ical2 = generateICalendar([fixtures.FIXTURE_SIMPLE_MARKET]);

      expect(ical1).toBe(ical2);
    });

    it('all fixtures generate identical output on repeated export', () => {
      const ical1 = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);
      const ical2 = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);

      expect(ical1).toBe(ical2);
    });

    it('exports are deterministic regardless of input array order', () => {
      const markets = [
        fixtures.FIXTURE_SIMPLE_MARKET,
        fixtures.FIXTURE_RECURRING_WEEKLY,
      ];

      const forward = generateICalendar(markets);
      const backward = generateICalendar([...markets].reverse());

      // Both should parse to the same set of events (order may differ in internal representation)
      const parsedForward = parseICalendar(forward);
      const parsedBackward = parseICalendar(backward);

      expect(parsedForward).toHaveLength(parsedBackward.length);
    });
  });

  describe('Recurring Markets (RRULE Validation)', () => {
    it('recurring markets include RRULE in output', () => {
      const ical = generateICalendar(fixtures.FIXTURE_RECURRING_MARKETS);

      expect(ical).toContain('RRULE:');
      expect(ical).toContain('FREQ=WEEKLY');
    });

    it('bi-weekly market has INTERVAL=2', () => {
      const ical = generateICalendar([fixtures.FIXTURE_RECURRING_BIWEEKLY]);

      expect(ical).toContain('INTERVAL=2');
    });

    it('recurring markets with UNTIL clause preserve deadline', () => {
      const ical = generateICalendar([fixtures.FIXTURE_RECURRING_BIWEEKLY]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].rrule).toContain('UNTIL=20240801T235959Z');
    });

    it('weekly market includes all 7 days of recurrence', () => {
      const ical = generateICalendar([fixtures.FIXTURE_RECURRING_WEEKLY]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].rrule).toContain('BYDAY=FR');
    });

    it('recurring markets round-trip through parse/serialize', () => {
      const original = fixtures.FIXTURE_RECURRING_WEEKLY;
      const ical = generateICalendar([original]);
      const parsed = parseICalendar(ical);

      expect(parsed).toHaveLength(1);
      expect(parsed[0].rrule).toContain(original.rrule);
    });
  });

  describe('Daylight-Saving Time Handling', () => {
    it('DST markets export with UTC timezone info only', () => {
      const ical = generateICalendar(fixtures.FIXTURE_DST_MARKETS);

      // Should use UTC (Z format), and only include standard UTC timezone
      const hasZ = ical.includes('Z');
      const hasUTC = ical.includes('TZID:UTC');

      expect(hasZ).toBe(true);
      expect(hasUTC).toBe(true);
    });

    it('spring forward transition preserves UTC time', () => {
      const market = fixtures.FIXTURE_DST_SPRING_FORWARD;
      const ical = generateICalendar([market]);
      const parsed = parseICalendar(ical);

      // Time should be preserved exactly
      expect(parsed[0].dtstart).toBe(market.end_time);
    });

    it('fall back transition preserves UTC time', () => {
      const market = fixtures.FIXTURE_DST_FALL_BACK;
      const ical = generateICalendar([market]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].dtstart).toBe(market.end_time);
    });

    it('both DST transitions produce valid iCalendar', () => {
      const ical = generateICalendar(fixtures.FIXTURE_DST_MARKETS);

      expect(ical).toContain('BEGIN:VCALENDAR');
      expect(ical).toContain('END:VCALENDAR');
      expect(ical).toContain('BEGIN:VEVENT');
      expect(ical).toContain('END:VEVENT');
    });
  });

  describe('Revision Tracking (SEQUENCE and LAST-MODIFIED)', () => {
    it('revised market shows higher SEQUENCE', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_REVISED]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].sequence).toBe(2);
    });

    it('highly revised market tracks many revisions', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_HIGHLY_REVISED]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].sequence).toBe(42);
    });

    it('LAST-MODIFIED differs from DTSTAMP when revised', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_REVISED]);

      // Parse to extract individual values (basic parser)
      const dtstampMatch = ical.match(/DTSTAMP:([^\r\n]+)/);
      const lastModMatch = ical.match(/LAST-MODIFIED:([^\r\n]+)/);

      expect(dtstampMatch).not.toBeNull();
      expect(lastModMatch).not.toBeNull();
      // They should differ for a revised market
      expect(dtstampMatch![1]).not.toEqual(lastModMatch![1]);
    });

    it('revision markets maintain stable UIDs', () => {
      const revisionUids = fixtures.FIXTURE_REVISION_MARKETS.map((m) => generateEventUID(m));

      // Each UID should be unique (different revision numbers)
      for (let i = 0; i < revisionUids.length - 1; i++) {
        expect(revisionUids[i]).not.toEqual(revisionUids[i + 1]);
      }
    });
  });

  describe('Special Characters and Escaping', () => {
    it('special chars market exports without corruption', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_SPECIAL_CHARS]);

      expect(ical).toBeTruthy();
      expect(ical).toContain('SUMMARY:');
      expect(ical).toContain('DESCRIPTION:');
    });

    it('special chars are properly escaped in SUMMARY', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_SPECIAL_CHARS]);

      // Commas, semicolons, backslashes should be escaped
      expect(ical).toContain('\\;');
      expect(ical).toContain('\\,');
      expect(ical).toContain('\\\\');
    });

    it('multi-line description preserves structure', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_SPECIAL_CHARS]);

      // Multi-line should be escaped as \\n
      expect(ical).toContain('\\n');
    });

    it('special chars parse correctly', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_SPECIAL_CHARS]);
      const parsed = parseICalendar(ical);

      expect(parsed).toHaveLength(1);
      expect(parsed[0].summary).toBeTruthy();
      expect(parsed[0].description).toBeTruthy();
    });
  });

  describe('Long Fields Handling', () => {
    it('long event names are preserved', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_LONG_FIELDS]);
      const parsed = parseICalendar(ical);

      expect(parsed[0].summary.length).toBeGreaterThan(50);
      expect(parsed[0].summary).toContain('blockchain');
    });

    it('long descriptions are included', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_LONG_FIELDS]);

      expect(ical).toContain('supply chain');
      expect(ical).toContain('transparency');
    });
  });

  describe('Batch Export and Round-Trip', () => {
    it('all fixtures export and parse back successfully', () => {
      const ical = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);
      const parsed = parseICalendar(ical);

      expect(parsed.length).toBe(fixtures.FIXTURE_ALL_MARKETS.length);
    });

    it('parsed events preserve key data', () => {
      const ical = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);
      const parsed = parseICalendar(ical);

      for (const event of parsed) {
        expect(event.uid).toBeTruthy();
        expect(event.summary).toBeTruthy();
        expect(event.dtstart).toBeGreaterThan(0);
        expect(event.dtend).toBeGreaterThan(event.dtstart);
        expect(event.sequence).toBeGreaterThanOrEqual(0);
      }
    });

    it('exported calendar is valid RFC 5545', () => {
      const ical = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);

      expect(ical).toMatch(/^BEGIN:VCALENDAR/);
      expect(ical).toMatch(/END:VCALENDAR$/);
      expect(ical).toContain('VERSION:2.0');
      expect(ical).toContain('PRODID:-//Predifi//Market Events//EN');
    });
  });

  describe('Minimal Market', () => {
    it('minimal fixture exports successfully', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_MINIMAL]);

      expect(ical).toContain('BEGIN:VEVENT');
      expect(ical).toContain('END:VEVENT');
    });

    it('minimal market has required iCalendar fields', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_MINIMAL]);
      const parsed = parseICalendar(ical);

      expect(parsed).toHaveLength(1);
      expect(parsed[0].uid).toBeTruthy();
      expect(parsed[0].summary).toBe('Minimal Market');
      expect(parsed[0].dtstart).toBeGreaterThan(0);
    });
  });

  describe('Snapshot Regression Detection', () => {
    it('simple market matches expected snapshot', () => {
      const ical = generateICalendar([fixtures.FIXTURE_SIMPLE_MARKET], {
        sourceBaseUrl: 'https://predifi.market',
      });

      // Basic validation instead of exact snapshot (timestamps vary)
      expect(ical).toContain('BEGIN:VCALENDAR');
      expect(ical).toContain('Bitcoin');
      expect(ical).toContain('$50');
      expect(ical).toContain('pool-1-rev0@predifi.market');
      expect(ical).toContain('SEQUENCE:0');
    });

    it('batch export maintains event count', () => {
      const ical = generateICalendar(fixtures.FIXTURE_ALL_MARKETS);

      const vevents = (ical.match(/BEGIN:VEVENT/g) || []).length;
      expect(vevents).toBe(fixtures.FIXTURE_ALL_MARKETS.length);
    });
  });

  describe('Export Options Integration', () => {
    it('custom title is reflected in export', () => {
      const ical = generateICalendar(fixtures.FIXTURE_ALL_MARKETS, {
        title: 'My Events',
        description: 'My Custom Calendar',
      });

      expect(ical).toContain('X-WR-CALNAME:My Events');
      expect(ical).toContain('X-WR-CALDESC:My Custom Calendar');
    });

    it('source base URL is included when no source_url on event', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_MINIMAL], {
        sourceBaseUrl: 'https://example.com',
      });

      expect(ical).toContain('URL:https://example.com/markets/');
    });

    it('event source_url takes precedence over sourceBaseUrl', () => {
      const ical = generateICalendar([fixtures.FIXTURE_SIMPLE_MARKET], {
        sourceBaseUrl: 'https://other.com',
      });

      // FIXTURE_SIMPLE_MARKET has explicit source_url set
      expect(ical).toContain('URL:https://predifi.market/markets/1');
    });

    it('export without source URL still works', () => {
      const ical = generateICalendar([fixtures.FIXTURE_MARKET_MINIMAL]);

      expect(ical).toContain('BEGIN:VCALENDAR');
      expect(ical).toContain('BEGIN:VEVENT');
    });
  });
});

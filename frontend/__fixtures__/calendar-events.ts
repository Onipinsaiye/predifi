/**
 * Deterministic test fixtures for calendar export validation.
 *
 * Fixtures include:
 * - Simple market close events
 * - Recurring market schedules
 * - Events across DST boundaries
 * - Revisions and cancellations
 * - High-volume export scenarios
 *
 * All timestamps are frozen for reproducible testing.
 */

import { MarketEvent } from '../src/lib/calendar' 

// Type re-export for convenience

/**
 * Frozen reference timestamp: 2024-01-01 00:00:00 UTC
 */
export const FIXTURE_BASE_TIME = 1704067200;

/**
 * Single market close event.
 * Represents a typical prediction market.
 */
export const FIXTURE_SIMPLE_MARKET: MarketEvent = {
  pool_id: 1,
  name: 'Will Bitcoin exceed $50,000?',
  description: 'Prediction market for BTC price target',
  category: 'Crypto',
  creator: 'alice.stellar',
  state: 'active',
  total_stake: '5000000000000000000', // 5.0 tokens
  end_time: FIXTURE_BASE_TIME + 86400, // +1 day
  created_at: FIXTURE_BASE_TIME,
  source_url: 'https://predifi.market/markets/1',
  revision: 0,
};

/**
 * Market with revision history.
 * Simulates an event that has been updated.
 */
export const FIXTURE_MARKET_REVISED: MarketEvent = {
  pool_id: 2,
  name: 'US Presidential Election 2024 Outcome',
  description: 'Prediction market for US presidential election',
  category: 'Politics',
  creator: 'bob.stellar',
  state: 'active',
  total_stake: '50000000000000000000', // 50.0 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 330, // ~11 months
  created_at: FIXTURE_BASE_TIME,
  revised_at: FIXTURE_BASE_TIME + 3600, // Revised 1 hour later
  source_url: 'https://predifi.market/markets/2',
  revision: 2, // Has been updated twice
};

/**
 * Recurring weekly market close.
 * Represents a market that runs every Friday until a specific date.
 */
export const FIXTURE_RECURRING_WEEKLY: MarketEvent = {
  pool_id: 3,
  name: 'Weekly Market: BTC Momentum',
  description: 'Weekly prediction on BTC price momentum',
  category: 'Crypto',
  creator: 'charlie.stellar',
  state: 'active',
  total_stake: '100000000000000000', // 0.1 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 5, // Friday of first week
  created_at: FIXTURE_BASE_TIME,
  rrule: 'FREQ=WEEKLY;BYDAY=FR',
  recurring_until: FIXTURE_BASE_TIME + 86400 * 365, // One year
  source_url: 'https://predifi.market/markets/3',
  revision: 0,
};

/**
 * Recurring bi-weekly market.
 * Uses explicit RRULE with UNTIL clause.
 */
export const FIXTURE_RECURRING_BIWEEKLY: MarketEvent = {
  pool_id: 4,
  name: 'Bi-weekly Market: ETH Trend',
  description: 'Bi-weekly prediction on ETH price trend',
  category: 'Crypto',
  creator: 'diana.stellar',
  state: 'active',
  total_stake: '200000000000000000', // 0.2 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 2, // First Monday
  created_at: FIXTURE_BASE_TIME,
  rrule: 'FREQ=WEEKLY;INTERVAL=2;BYDAY=MO',
  rrule_until: '20240801T235959Z', // Ends Aug 1, 2024
  source_url: 'https://predifi.market/markets/4',
  revision: 0,
};

/**
 * Market across daylight-saving time transition (US Eastern, spring forward).
 * March 12, 2024: 2:00 AM EDT becomes 3:00 AM EDT
 * This market close happens right at the transition.
 */
export const FIXTURE_DST_SPRING_FORWARD: MarketEvent = {
  pool_id: 5,
  name: 'Spring Forward Test Market',
  description: 'Market close during DST transition (spring forward)',
  category: 'Test',
  creator: 'test.stellar',
  state: 'active',
  total_stake: '1000000000000000', // 0.001 tokens
  end_time: 1710270000, // March 12, 2024, 6:00 AM UTC (2:00 AM EDT before transition)
  created_at: FIXTURE_BASE_TIME,
  source_url: 'https://predifi.market/markets/5',
  revision: 0,
};

/**
 * Market across daylight-saving time transition (US Eastern, fall back).
 * November 5, 2024: 1:00 AM EDT becomes 12:00 AM EST
 * Market close happens during this transition.
 */
export const FIXTURE_DST_FALL_BACK: MarketEvent = {
  pool_id: 6,
  name: 'Fall Back Test Market',
  description: 'Market close during DST transition (fall back)',
  category: 'Test',
  creator: 'test.stellar',
  state: 'active',
  total_stake: '1000000000000000', // 0.001 tokens
  end_time: 1730800800, // November 5, 2024, 5:00 AM UTC (1:00 AM EDT before transition)
  created_at: FIXTURE_BASE_TIME,
  source_url: 'https://predifi.market/markets/6',
  revision: 0,
};

/**
 * Market with maximum revision number.
 * Simulates a heavily-updated market over its lifecycle.
 */
export const FIXTURE_MARKET_HIGHLY_REVISED: MarketEvent = {
  pool_id: 7,
  name: 'Long-running Market with Many Updates',
  description: 'Market that has been updated many times',
  category: 'Test',
  creator: 'eve.stellar',
  state: 'active',
  total_stake: '1000000000000000000', // 1.0 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 30,
  created_at: FIXTURE_BASE_TIME,
  revised_at: FIXTURE_BASE_TIME + 86400 * 29, // Last revised ~29 days later
  source_url: 'https://predifi.market/markets/7',
  revision: 42, // High revision count
};

/**
 * Market with special characters in name and description.
 * Tests escaping logic (commas, semicolons, backslashes, newlines).
 */
export const FIXTURE_MARKET_SPECIAL_CHARS: MarketEvent = {
  pool_id: 8,
  name: 'Test; Market, with\\special;chars',
  description: 'Description line 1\nDescription line 2\nDescription line 3',
  category: 'Test',
  creator: 'frank.stellar',
  state: 'active',
  total_stake: '500000000000000000', // 0.5 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 7,
  created_at: FIXTURE_BASE_TIME,
  source_url: 'https://predifi.market/markets/8',
  revision: 0,
};

/**
 * Market with very long name and description.
 * Tests handling of large field values.
 */
export const FIXTURE_MARKET_LONG_FIELDS: MarketEvent = {
  pool_id: 9,
  name: 'Will blockchain technology revolutionize supply chain management and logistics across global markets in the next five years?',
  description:
    'This market examines the adoption and impact of blockchain technology in supply chain and logistics. ' +
    'Key indicators include: enterprise adoption rate, cost reductions, transparency improvements, and regulatory framework development. ' +
    'The prediction period extends 5 years from market creation, covering technological maturation and real-world deployment at scale.',
  category: 'Technology',
  creator: 'grace.stellar',
  state: 'active',
  total_stake: '10000000000000000000', // 10.0 tokens
  end_time: FIXTURE_BASE_TIME + 86400 * 1825, // 5 years
  created_at: FIXTURE_BASE_TIME,
  source_url: 'https://predifi.market/markets/9',
  revision: 0,
};

/**
 * Market without optional fields.
 * Minimal event for testing robustness.
 */
export const FIXTURE_MARKET_MINIMAL: MarketEvent = {
  pool_id: 10,
  name: 'Minimal Market',
  end_time: FIXTURE_BASE_TIME + 3600,
  created_at: FIXTURE_BASE_TIME,
};

/**
 * Collection of all fixture markets for batch testing.
 */
export const FIXTURE_ALL_MARKETS: MarketEvent[] = [
  FIXTURE_SIMPLE_MARKET,
  FIXTURE_MARKET_REVISED,
  FIXTURE_RECURRING_WEEKLY,
  FIXTURE_RECURRING_BIWEEKLY,
  FIXTURE_DST_SPRING_FORWARD,
  FIXTURE_DST_FALL_BACK,
  FIXTURE_MARKET_HIGHLY_REVISED,
  FIXTURE_MARKET_SPECIAL_CHARS,
  FIXTURE_MARKET_LONG_FIELDS,
  FIXTURE_MARKET_MINIMAL,
];

/**
 * Markets suitable for DST transition testing.
 */
export const FIXTURE_DST_MARKETS: MarketEvent[] = [
  FIXTURE_DST_SPRING_FORWARD,
  FIXTURE_DST_FALL_BACK,
];

/**
 * Markets suitable for revision tracking testing.
 */
export const FIXTURE_REVISION_MARKETS: MarketEvent[] = [
  FIXTURE_MARKET_REVISED,
  FIXTURE_MARKET_HIGHLY_REVISED,
];

/**
 * Markets suitable for recurrence testing.
 */
export const FIXTURE_RECURRING_MARKETS: MarketEvent[] = [
  FIXTURE_RECURRING_WEEKLY,
  FIXTURE_RECURRING_BIWEEKLY,
];

/**
 * Pre-generated expected iCalendar output for the simple market.
 * Used for snapshot testing and regression detection.
 */
export const FIXTURE_ICAL_SIMPLE_MARKET = `BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Predifi//Market Events//EN\r
CALSCALE:UTC\r
METHOD:PUBLISH\r
X-PREDIFI-EXPORTED:20240101T000000\r
NAME:Predifi Market Events\r
X-WR-CALNAME:Predifi Market Events\r
DESCRIPTION:Prediction market close times and recurring events\r
X-WR-CALDESC:Prediction market close times and recurring events\r
X-WR-TIMEZONE:UTC\r
BEGIN:VTIMEZONE\r
TZID:UTC\r
BEGIN:STANDARD\r
DTSTART:19700101T000000Z\r
TZOFFSETFROM:+0000\r
TZOFFSETTO:+0000\r
TZNAME:UTC\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:pool-1-rev0@predifi.market\r
DTSTAMP:20240101T000000Z\r
DTSTART:20240102T000000Z\r
DTEND:20240102T010000Z\r
SUMMARY:Will Bitcoin exceed $50,000?\r
DESCRIPTION:Prediction market for BTC price target\nStatus: active\nTotal Stake: 5.00\nCreated by: alice.stellar\nCategory: Crypto\r
SEQUENCE:0\r
LAST-MODIFIED:20240101T000000Z\r
STATUS:CONFIRMED\r
URL:https://predifi.market/markets/1\r
TRANSP:OPAQUE\r
CLASS:PUBLIC\r
END:VEVENT\r
END:VCALENDAR`;

/**
 * Pre-generated expected iCalendar output for recurring markets.
 * Used for snapshot testing of RRULE handling.
 */
export const FIXTURE_ICAL_RECURRING_MARKETS = `BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Predifi//Market Events//EN\r
CALSCALE:UTC\r
METHOD:PUBLISH\r
X-PREDIFI-EXPORTED:20240101T000000\r
NAME:Predifi Market Events\r
X-WR-CALNAME:Predifi Market Events\r
DESCRIPTION:Prediction market close times and recurring events\r
X-WR-CALDESC:Prediction market close times and recurring events\r
X-WR-TIMEZONE:UTC\r
BEGIN:VTIMEZONE\r
TZID:UTC\r
BEGIN:STANDARD\r
DTSTART:19700101T000000Z\r
TZOFFSETFROM:+0000\r
TZOFFSETTO:+0000\r
TZNAME:UTC\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:pool-3-rev0@predifi.market\r
DTSTAMP:20240101T000000Z\r
DTSTART:20240105T000000Z\r
DTEND:20240105T010000Z\r
SUMMARY:Weekly Market: BTC Momentum\r
DESCRIPTION:Weekly prediction on BTC price momentum\nStatus: active\nTotal Stake: 0.10\nCreated by: charlie.stellar\nCategory: Crypto\r
SEQUENCE:0\r
LAST-MODIFIED:20240101T000000Z\r
STATUS:CONFIRMED\r
RRULE:FREQ=WEEKLY;BYDAY=FR;UNTIL=20250101T000000Z\r
URL:https://predifi.market/markets/3\r
TRANSP:OPAQUE\r
CLASS:PUBLIC\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:pool-4-rev0@predifi.market\r
DTSTAMP:20240101T000000Z\r
DTSTART:20240103T000000Z\r
DTEND:20240103T010000Z\r
SUMMARY:Bi-weekly Market: ETH Trend\r
DESCRIPTION:Bi-weekly prediction on ETH price trend\nStatus: active\nTotal Stake: 0.20\nCreated by: diana.stellar\nCategory: Crypto\r
SEQUENCE:0\r
LAST-MODIFIED:20240101T000000Z\r
STATUS:CONFIRMED\r
RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO;UNTIL=20240801T235959Z\r
URL:https://predifi.market/markets/4\r
TRANSP:OPAQUE\r
CLASS:PUBLIC\r
END:VEVENT\r
END:VCALENDAR`;

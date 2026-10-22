import fc from "fast-check";
import {
  formatDeadline,
  fromUnixSeconds,
  parseDeadline,
  type DeadlineInput,
} from "@/lib/deadline/format-deadline";

/**
 * Property-based tests for deadline formatting using fast-check.
 * Ensures timezone handling is robust across a wide range of inputs and timezones.
 */
describe("formatDeadline property-based tests", () => {
  const majorTimezones = [
    "UTC",
    "America/New_York",
    "America/Los_Angeles",
    "Europe/London",
    "Europe/Paris",
    "Asia/Tokyo",
    "Asia/Shanghai",
    "Asia/Kolkata",
    "Australia/Sydney",
    "Pacific/Kiritimati",
  ];

  const majorLocales = ["en-US", "de-DE", "fr-FR", "ja-JP", "zh-CN", "es-ES"];

  describe("parseDeadline accepts all valid ISO-8601 with offset", () => {
    it("accepts valid ISO strings with Z suffix", () => {
      fc.assert(
        fc.property(fc.integer({ min: -62135596800000, max: 253402300799999 }), (ms) => {
          const date = new Date(ms);
          const isoString = date.toISOString();
          const parsed = parseDeadline(isoString);
          expect(parsed.status).toBe("valid");
          if (parsed.status === "valid") {
            expect(parsed.date.getTime()).toBe(ms);
          }
        })
      );
    });

    it("accepts Date objects", () => {
      fc.assert(
        fc.property(fc.integer({ min: -62135596800000, max: 253402300799999 }), (ms) => {
          const date = new Date(ms);
          const parsed = parseDeadline(date);
          expect(parsed.status).toBe("valid");
          if (parsed.status === "valid") {
            expect(parsed.date.getTime()).toBe(ms);
          }
        })
      );
    });

    it("accepts epoch milliseconds", () => {
      fc.assert(
        fc.property(fc.integer({ min: 0, max: 9999999999999 }), (ms) => {
          const parsed = parseDeadline(ms);
          expect(parsed.status).toBe("valid");
          if (parsed.status === "valid") {
            expect(parsed.date.getTime()).toBe(ms);
          }
        })
      );
    });
  });

  describe("formatDeadline produces consistent UTC absolute time", () => {
    it("absolute UTC is the same regardless of timezone or locale", () => {
      fc.assert(
        fc.property(
          fc.integer({ min: -62135596800000, max: 253402300799999 }),
          fc.sample(fc.constantFrom(...majorTimezones), 2, 2)[0],
          fc.sample(fc.constantFrom(...majorLocales), 2, 2)[0]
        ),
        (ms, tz, locale) => {
          const date = new Date(ms);
          const f1 = formatDeadline(date, { timeZone: tz, locale });
          const f2 = formatDeadline(date, { timeZone: majorTimezones[0], locale: majorLocales[0] });

          // Absolute UTC should be identical
          expect(f1.absolute).toBe(f2.absolute);
        }
      );
    });
  });

  describe("formatDeadline respects locale in local time", () => {
    it("locale affects date/time representation", () => {
      fc.assert(
        fc.property(
          fc.integer({ min: -62135596800000, max: 253402300799999 }),
          (ms) => {
            const date = new Date(ms);
            const enUs = formatDeadline(date, { locale: "en-US", timeZone: "UTC" });
            const deDe = formatDeadline(date, { locale: "de-DE", timeZone: "UTC" });
            const jaJp = formatDeadline(date, { locale: "ja-JP", timeZone: "UTC" });

            // Different locales should produce different text representations
            // (at least one should differ from another)
            const texts = [enUs.local, deDe.local, jaJp.local];
            const uniqueTexts = new Set(texts);
            expect(uniqueTexts.size).toBeGreaterThan(1);
          }
        )
      );
    });
  });

  describe("formatDeadline respects timezone in local time", () => {
    it("timezone affects the wall-clock time shown", () => {
      fc.assert(
        fc.property(fc.integer({ min: 0, max: 9999999999999 }), (ms) => {
          const date = new Date(ms);
          const utc = formatDeadline(date, { timeZone: "UTC", locale: "en-US" });
          const ny = formatDeadline(date, { timeZone: "America/New_York", locale: "en-US" });

          // Unless it's exactly midnight, times should differ
          if (date.getUTCHours() !== 0) {
            expect(utc.local).not.toBe(ny.local);
          }

          // Absolute should be the same
          expect(utc.absolute).toBe(ny.absolute);
        })
      );
    });
  });

  describe("remainingMs calculation is consistent", () => {
    it("remainingMs = deadline - now for all valid inputs", () => {
      fc.assert(
        fc.property(
          fc.integer({ min: 0, max: 9999999999999 }),
          fc.integer({ min: 0, max: 9999999999999 })
        ),
        (deadlineMs, nowMs) => {
          const formatted = formatDeadline(deadlineMs, { now: nowMs });

          if (formatted.status !== "missing" && formatted.status !== "invalid") {
            const expected = deadlineMs - nowMs;
            expect(formatted.remainingMs).toBe(expected);
          }
        }
      );
    });
  });

  describe("status transitions", () => {
    it("status is 'open', 'closing-soon', or 'expired' based on remaining time", () => {
      fc.assert(
        fc.property(
          fc.integer({ min: 0, max: 9999999999999 }),
          fc.integer({ min: 0, max: 9999999999999 })
        ),
        (deadlineMs, nowMs) => {
          const formatted = formatDeadline(deadlineMs, { now: nowMs });

          if (formatted.remainingMs !== null) {
            const remaining = formatted.remainingMs;
            if (remaining > 3600 * 1000) {
              // More than 1 hour
              expect(formatted.status).toBe("open");
            } else if (remaining > 0) {
              // Between 0 and 1 hour
              expect(["open", "closing-soon"]).toContain(formatted.status);
            } else {
              // Expired
              expect(formatted.status).toBe("expired");
            }
          }
        }
      );
    });
  });

  describe("fromUnixSeconds converter", () => {
    it("correctly converts Unix seconds to milliseconds", () => {
      fc.assert(
        fc.property(fc.integer({ min: -62135596800, max: 253402300799 }), (seconds) => {
          const ms = fromUnixSeconds(seconds);
          expect(ms).toBe(seconds * 1000);
        })
      );
    });
  });

  describe("fallback to default locale/timezone", () => {
    it("invalid timezone falls back to UTC", () => {
      fc.assert(
        fc.property(fc.integer({ min: 0, max: 9999999999999 }), (ms) => {
          const formatted = formatDeadline(ms, {
            timeZone: "Invalid/Timezone",
            locale: "en-US",
          });
          expect(formatted.timeZone).toBe("UTC");
        })
      );
    });

    it("invalid locale falls back to en-US", () => {
      fc.assert(
        fc.property(fc.integer({ min: 0, max: 9999999999999 }), (ms) => {
          const formatted = formatDeadline(ms, {
            locale: "invalid_locale!!!",
            timeZone: "UTC",
          });
          expect(formatted.locale).toBe("en-US");
        })
      );
    });
  });

  describe("timezone matrix across major zones", () => {
    it("all major timezones format successfully", () => {
      const testDate = new Date("2026-06-15T12:00:00Z");

      fc.assert(
        fc.property(
          fc.integer({ min: 0, max: majorTimezones.length - 1 }),
          (tzIndex) => {
            const tz = majorTimezones[tzIndex];
            const formatted = formatDeadline(testDate, { timeZone: tz, locale: "en-US" });

            expect(formatted.status).toBe("unknown"); // No clock provided
            expect(formatted.local).not.toBeNull();
            expect(formatted.absolute).toBe("2026-06-15T12:00:00Z");
            expect(formatted.timeZone).toBe(tz);
          }
        )
      );
    });
  });
});

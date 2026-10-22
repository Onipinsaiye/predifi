# Testing Guide: Timezone Formatting & Optimistic Market Updates

This document provides comprehensive testing guidance for issues #80 and #72.

---

## Issue #80: Standardize Timezone and Deadline Formatting

### What's Being Tested

The deadline formatting system ensures that pool cutoff times are displayed consistently across all user locales and timezones without confusion.

#### Core Guarantees
1. **Locale-Aware Display**: Times shown in user's local timezone with abbreviation (EDT, PST, etc.)
2. **Unambiguous Reference**: UTC instant always provided to prevent confusion
3. **Hydration Safety**: Server-rendered content never mismatches client-rendered content
4. **DST Handling**: Correctly handles daylight-saving transitions across different regions
5. **Status Tracking**: Deadline state tracked as open, closing-soon, or expired

### Test Files

#### `__tests__/format-deadline.test.ts`
Tests the core `formatDeadline()` function.

**Key Test Cases**:
```typescript
// Parsing various input formats
parseDeadline(null)                              // → "missing"
parseDeadline("garbage")                        // → "invalid"
parseDeadline("2026-03-08T07:00:00Z")          // → "valid"
parseDeadline("2026-03-08T08:00:00+01:00")     // → "valid"
parseDeadline(new Date())                       // → "valid"

// Timezone matrix
formatDeadline(instant, { timeZone: "UTC", locale: "en-US" })
formatDeadline(instant, { timeZone: "America/New_York", locale: "en-US" })
formatDeadline(instant, { timeZone: "Europe/Berlin", locale: "de-DE" })

// DST transitions
formatDeadline("2026-03-08T06:59:00Z", { timeZone: "America/New_York" })  // 1:59 AM EST
formatDeadline("2026-03-08T07:00:00Z", { timeZone: "America/New_York" })  // 3:00 AM EDT

// Boundary conditions
formatDeadline(deadline, { now: deadline - 1 }).status                    // "closing-soon"
formatDeadline(deadline, { now: deadline }).status                        // "expired"
```

#### `__tests__/use-deadline.test.ts`
Tests the React hook that handles hydration safety and ticking.

**Key Test Cases**:
```typescript
// Hydration safety: SSR render
const { result } = renderHook(() => useDeadline(deadline))
result.current.ready  // false
result.current.status // "unknown"
result.current.local  // null (deterministic SSR)

// After hydration
await waitFor(() => expect(result.current.ready).toBe(true))
result.current.status // "open" or "closing-soon"
result.current.local  // "Mar 8, 2026, 3:00 AM EDT"

// Ticking interval
// Ticks every 1s when "closing-soon"
// Ticks every 30s when "open"
// Doesn't tick when "expired" or "unknown"
```

#### `__tests__/pool-deadline.test.tsx`
Tests the PoolDeadline component integration.

**Key Test Cases**:
```typescript
// Rendering states
render(<PoolDeadline endTime={null} />)              // "Deadline not set"
render(<PoolDeadline endTime={futureSeconds} />)    // Shows time + countdown
render(<PoolDeadline endTime={pastSeconds} />)      // Shows "Closed"

// Format accuracy
// - Includes timezone abbreviation
// - Shows UTC reference with (UTC)
// - Uses semantic <time> element
// - Custom label support
```

#### `__tests__/format-deadline.property.test.ts`
Property-based tests using fast-check for exhaustive coverage.

**Test Strategy**:
```typescript
// All ISO strings with offset should parse to same instant
fc.property(fc.string(), (isoString) => {
  const parsed = parseDeadline(isoString)
  // Verify consistency
})

// UTC absolute time should be identical across timezones
fc.property(majorTimezones, (tz) => {
  const f1 = formatDeadline(date, { timeZone: tz })
  const f2 = formatDeadline(date, { timeZone: "UTC" })
  expect(f1.absolute).toBe(f2.absolute)
})

// Status should match remaining time
fc.property(fc.integer(), (remaining) => {
  if (remaining > 60*60*1000) expect(status).toBe("open")
  if (remaining <= 0) expect(status).toBe("expired")
})
```

### Running Deadline Tests

```bash
# All deadline tests
npm test -- --testPathPattern="deadline"

# Specific test file
npm test -- __tests__/format-deadline.test.ts
npm test -- __tests__/use-deadline.test.ts
npm test -- __tests__/pool-deadline.test.tsx

# Property-based tests only (exhaustive)
npm test -- --testPathPattern="property.test"

# With coverage report
npm test -- --testPathPattern="deadline" --coverage
```

### Key Scenarios to Test Manually

1. **Timezone Switch**
   - Navigate to a pool detail page
   - Change system timezone
   - Verify deadline time updates immediately
   - UTC reference should not change

2. **DST Transition**
   - Set system clock to 2:00 AM on US spring-forward day
   - Navigate to pool closing at 2:30 AM (which becomes 3:30 AM EDT)
   - Should show correct EDT time
   - Same for fall-back transition

3. **Multiple Locales**
   - Test with `navigator.language` = en-US, de-DE, ja-JP, etc.
   - Deadline should render in locale's date/time format
   - UTC reference should always be ISO-8601 format

4. **Countdown Edge Cases**
   - Create pool with 1 minute deadline
   - Watch countdown: 59s, 58s, ..., 1s, 0s
   - At 0s, should show "Closed"
   - Verify countdown stops updating

---

## Issue #72: Optimistic Market Updates with Authoritative Rollback

### What's Being Tested

The optimistic update system ensures that users see immediate feedback for their predictions while maintaining data consistency and rolling back on failure.

#### Core Guarantees
1. **Optimistic Feedback**: Stake totals and odds update immediately on prediction
2. **Authoritative Reconciliation**: Server-provided odds stay authoritative
3. **Error Recovery**: Failed predictions roll back cleanly
4. **Concurrent Safety**: Multiple simultaneous predictions handled correctly
5. **No Data Reversion**: Newer confirmed data never overwritten by stale responses

### Test Files

#### `__tests__/pool-detail-view.test.tsx`
Tests the pool detail component with live WebSocket and optimistic updates.

**Key Test Cases**:
```typescript
// WebSocket live update
const event: PoolLiveEvent = {
  type: "prediction_placed",
  pool_id: 1,
  outcome: 0,
  amount: 100
}
// After event: total_stake should increase by 100

// Optimistic rollback
const commit = jest.fn().mockRejectedValue(new Error("Network error"))
result.current.run(prediction)  // Optimistic: +100
// After error: total_stake rolls back to original

// Stale response handling
request1.response = { total_stake: 1200 }  // Fresh
request2.response = { total_stake: 1100 }  // Stale (arrives later)
// Final state: 1200 (stale never overwrites fresh)
```

#### `__tests__/optimistic-market-updates.test.ts`
Tests optimistic action hook with market-specific scenarios.

**Key Test Cases**:
```typescript
// Single market update
run({ amount: 100, outcome: 0 })
// Immediately: value.total_stake = 1100
// Optimistic until commit succeeds

// Concurrent updates on same market
run({ amount: 100, outcome: 0 })
run({ amount: 50, outcome: 1 })  // While first pending
// Both applied optimistically
// Final: total_stake = 1150

// Overlapping failure and success
run({ amount: 100 })  // Returns error
run({ amount: 50 })   // Returns success
// First rolled back, second confirmed
// Final: total_stake = 1050 (only second applied)

// Route navigation during pending
run({ amount: 100 })  // Pending...
unmount()             // Component unmounts
// No state updates after unmount
// Transaction may or may not complete server-side
```

### Running Market Update Tests

```bash
# All optimistic update tests
npm test -- --testPathPattern="optimistic|pool-detail"

# Specific test file
npm test -- __tests__/pool-detail-view.test.tsx
npm test -- __tests__/optimistic-market-updates.test.ts

# With coverage
npm test -- --testPathPattern="optimistic" --coverage

# Watch mode (re-runs on file change)
npm test -- --testPathPattern="optimistic" --watch
```

### Key Scenarios to Test Manually

1. **Fast Consecutive Predictions**
   - Click "Predict" on outcome A
   - Immediately click "Predict" on outcome B
   - Verify both stakes increase in UI
   - Wait for confirmations
   - Verify both are confirmed without revert

2. **Network Failure and Retry**
   - Disable network
   - Click "Predict"
   - See optimistic update
   - Wait 10+ seconds
   - Should show error toast
   - Verify stake rolled back
   - Re-enable network
   - Retry prediction
   - Should succeed

3. **Stale Response Scenario**
   - Create rapid predictions (A, B, C)
   - Simulate B response arriving out of order (late)
   - Verify C state is not overwritten by B

4. **Route Change During Prediction**
   - Click "Predict"
   - See optimistic update
   - Immediately navigate away
   - Navigate back to pool
   - Should see data refreshed from server
   - No orphaned optimistic state

5. **WebSocket Disconnect**
   - Disconnect WebSocket (browser DevTools)
   - Try to make prediction
   - Should fallback to API
   - Reconnect WebSocket
   - Should resume live updates

### Understanding the Flow

#### Successful Prediction Flow
```
1. User clicks "Predict $100 on Outcome A"
   ↓
2. useOptimisticAction.run() called
   ↓
3. Optimistic update applied immediately
   - total_stake: 1000 → 1100
   - outcome_a.stake: 600 → 700
   - Odds recomputed
   ↓
4. UI updates (animated number change)
   ↓
5. Transaction sent to blockchain
   ↓
6. Transaction confirmed/indexed
   ↓
7. onSuccess() callback fires
   - SWR mutate() called
   - Server odds fetched (authoritative)
   - Client odds updated if changed
   ↓
8. UI remains at confirmed state
```

#### Failed Prediction Flow
```
1. User clicks "Predict $100 on Outcome A"
   ↓
2. Optimistic update applied (same as above)
   ↓
3. Transaction sent
   ↓
4. Transaction fails (user rejected, insufficient balance, etc.)
   ↓
5. Error event received
   ↓
6. Rollback triggered
   - Optimistic state dropped
   - Falls back to last confirmed data
   - total_stake: 1100 → 1000
   - outcome_a.stake: 700 → 600
   ↓
7. Error toast shown to user
   ↓
8. User can retry after fixing issue
```

---

## Debugging Tips

### Timezone Issues

**Problem**: Deadline shows wrong time
```typescript
// Check which timezone is being used
console.log(new Intl.DateTimeFormat().resolvedOptions().timeZone)

// Check Intl formatting directly
const formatter = new Intl.DateTimeFormat('en-US', {
  year: 'numeric',
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
  timeZone: 'America/New_York'
})
console.log(formatter.format(new Date()))
```

**Problem**: Hydration mismatch
```typescript
// Check if useDeadline ready flag is true
// SSR should have ready=false, status="unknown"
// Client after hydration should have ready=true, status="open"/"closing-soon"/"expired"
```

### Optimistic Update Issues

**Problem**: Stake doesn't update immediately
```typescript
// Verify useOptimisticAction is called
// Check that applyOptimistic function is correct
// Verify component is using value from hook result

// Check SWR cache
console.log('SWR cache:', useSWRConfig()._cache)
```

**Problem**: Data reverted unexpectedly
```typescript
// Check if newer response arrived after stale
// Add logging to see response timestamps
// Verify stale response filtering logic
// Check if mutate() was called at right time
```

**Problem**: Multiple predictions cause issues
```typescript
// Verify each run() call completes independently
// Check that optimistic value is based on current confirmed data
// Ensure state updates are not batched incorrectly
```

---

## Performance Considerations

### Deadline Formatting
- `useDeadline()` sets intervals (1s or 30s)
- Multiple deadlines should be efficient (share interval?)
- Consider bundling deadline tickers

### Optimistic Updates
- `setLive()` updates trigger re-renders
- Animated number changes every update
- Large stake numbers may cause jank
- Consider debouncing live updates

---

## Accessibility Considerations

### Deadline Formatting
- ✅ Uses semantic `<time>` element with `dateTime` attribute
- ✅ Text content readable by screen readers
- ✅ Timezone abbreviation included in aria-label?
- ❓ Consider aria-live for countdown updates

### Optimistic Updates
- ✅ Animated numbers use `<AnimatedNumber>` component
- ✅ Error toast provides feedback
- ✅ No hidden state changes
- ❓ Consider aria-busy during pending state

---

## Common Issues & Resolutions

| Issue | Cause | Resolution |
|-------|-------|-----------|
| Deadline shows wrong time | Timezone mismatch | Check `Intl.DateTimeFormat().resolvedOptions().timeZone` |
| Countdown stops updating | Interval not set | Verify `useDeadline` hook status is not "expired" |
| Hydration mismatch error | SSR uses different data | Verify useDeadline ready flag logic |
| Stake doesn't revert on error | Rollback not triggered | Check error handling in useOptimisticAction |
| Multiple predictions fail | Race condition | Verify each prediction is independent |
| Stale data overwrites fresh | Response ordering | Check timestamp comparison in handler |
| WebSocket events ignored | Pool ID mismatch | Verify poolId in event matches current pool |

---

## CI/CD Integration

### GitHub Actions / Similar

```yaml
- name: Test Deadline Formatting
  run: npm test -- --testPathPattern="deadline" --coverage

- name: Test Optimistic Updates
  run: npm test -- --testPathPattern="optimistic|pool-detail"

- name: Property-Based Tests
  run: npm test -- --testPathPattern="property.test"

- name: Upload Coverage
  uses: codecov/codecov-action@v3
  with:
    files: ./coverage/lcov.info
```

---

## References

- [MDN: Intl.DateTimeFormat](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Intl/DateTimeFormat)
- [React Testing Library](https://testing-library.com/react)
- [Jest Documentation](https://jestjs.io/)
- [fast-check Property Testing](https://github.com/dubzzz/fast-check)
- [SWR Documentation](https://swr.vercel.app/)
- [useOptimisticAction Hook](../lib/hooks/useOptimisticAction.ts)

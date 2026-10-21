# Accessibility (WCAG 2.1 AA)

Covers [#1389](https://github.com/Web3Novalabs/predifi/issues/1389).

Target: **WCAG 2.1 Level AA**. This page records what the audit changed, the
primitives now available, and the checklist to apply when building new UI.

## Primitives

All exported from `@/components/ui`.

| Primitive | Use it for |
| --- | --- |
| `<SkipLink />` | Bypassing the nav. Rendered first in `app/layout.tsx`, so it is the first tab stop; visible only on focus. Targets `#main-content`. |
| `<VisuallyHidden>` | Context that sighted users get from layout or icons — icon-only button labels, table column meanings. Pass `focusable` to reveal on focus. |
| `<LiveRegionProvider> / useAnnounce()` | Announcing changes that do not move focus: prediction placed, claim confirmed, odds updated. Wraps the app in `layout.tsx`. |
| `<SkeletonScreen>` | Loading placeholders. Owns the single `role="status"` announcement so the individual bars stay `aria-hidden`. |

```tsx
const announce = useAnnounce();

await placePrediction(input);
announce("Prediction placed: 50 XLM on Yes");   // polite (default)
announce("Transaction failed", "assertive");     // interrupts
```

## Transaction and validation announcements

The wallet, submission, confirmation, and form-validation flows convey their
state through live regions so screen-reader users do not have to inspect
visual status changes.

- **Wallet connection** — connecting, connected, disconnected, and failure
outcomes go through `useAnnounce()`. Connection failures use the assertive
politeness so they interrupt rather than wait behind other queued messages.
- **Submission and confirmation** — pending → submitted → confirmed/failed transitions
are announced once per state change. The pending and confirming states are
polite; terminal failures are assertive.
- **Polling deduplication** — polling updates (odds, balance, confirmation
polls) must not re-announce the same message on every tick. Announce only when the
displayed value or status actually changes, and dedupe identical consecutive
messages in the live region.
- `aria-busy="true"` is on the region that owns the pending state and is
cleared when the region settles, so assistive technology knows the update is
still in flight.

## Form validation

- Every input has a programmatic `<label>`.
- Error messages are rendered in a live region and linked to the input via
`aria-describedby`; the input sets `aria-invalid="true"` while the error is present.
- On submit failure, focus is moved to the first invalid field (or to the error
summary) and the error is announced. Focus is preserved on the field the user
was editing rather than being blurted to the top of the page.

## Testing announcements

Tests query by accessible role and accessible name rather than by internal
selectors:

- **Status announcements** — query the live region with `getByRole("status")`
or `getBrole("alert")` and assert on its text content after the state change.
- **Polling dedup** — advance timers through several polling ticks with an
unchanged value and assert the live region text does not change.
- **Form error association** — query the input by its accessible name, assert
`aria-invalid="true"`, and confirm the error text is resolved through
`aria-describedby`.
- **Focus preservation** — assert document.activeElement is the expected field
or the error summary after a failed submit.

## What the audit changed

- **Bypass blocks (2.4.1)** — added `<SkipLink />` to the root layout and
`id="main-content"` + `tabIndex={-1}` to the `<main>` landmark on the home,
about, settings, and waitlist pages.
- **Status messages (4.1.3)** — added polite and assertive live regions so
prediction and claim outcomes are announced without stealing focus. Wallet
connection, transaction submission/confirmation, and form validation now follow
the same pattern.
- **Loading states (4.1.3, 1.3.1)** — skeletons announce once via
`role="status"` + `aria-busy` instead of exposing dozens of empty nodes.
- **Motion (2.3.3)** — skeleton pulse and shimmer are behind Tailwind's
`motion-safe:` prefix, so `prefers-reduced-motion: reduce` stops them.

## Checklist for new UI

**Keyboard (2.1.1, 2.4.3, 2.4.7)**
- [ ] Every interactive element is reachable and operable with Tab / Shift+Tab / Enter / Space.
- [ ] Tab order follows visual order; nothing is a keyboard trap.
- [ ] Focus is always visible — never remove the ring without replacing it (`focus-visible:ring-2`).
- [ ] Modals trap focus while open, close on Escape, and restore focus to the trigger.
- [ ] Custom controls built on `<div>` need `role`, `tabIndex={0}`, and Enter/Space handlers. Prefer a real `<button>`.

**Names and structure (1.3.1, 2.4.6, 4.1.2)**
- [ ] Icon-only buttons have `aria-label` or a `<VisuallyHidden>` child.
- [ ] Headings descend without skipping levels; one `<h1>` per page.
- [ ] Landmarks in place: `<header>`, `<nav>`, `<main id="main-content">`, `<footer>`.
- [ ] Form inputs have an associated `<label>`; errors use `aria-describedby` and `aria-invalid`.
- [ ] Data tables use `<th scope="col">` / `<th scope="row">`.

**Dynamic content (4.1.3)**
- [ ] Async results are announced via `useAnnounce()`.
- [ ] Wallet connection, request submission, and confirmation states are announced once per change.
- [ ] Polling updates are deduplicated and only announce when the value changes.
- [ ] Loading regions set `aria-busy="true"`; placeholders are `aria-hidden`.
- [ ] Toasts render inside a live region.

**Colour and contrast (1.4.3, 1.4.11, 1.4.1)**
- [ ] Text contrast ≥ 4.5:1 (≥ 3:1 for text ≥ 24px or ≥ 19px bold).
- [ ] UI component and focus-indicator contrast ≥ 3:1.
- [ ] Colour is never the only signal — pair win/loss and up/down with an icon or text.

**Content and motion (1.4.4, 1.4.10, 2.3.3, 1.1.1)**
- [ ] Layout survives 200% zoom and a 320px viewport without horizontal scroll.
- [ ] Animation respects `prefers-reduced-motion` (`motion-safe:` / `motion-reduce:`).
- [ ] Images have alt`; decorative images use `alt=""`.

## Verifying

```bash
cd frontend && pnpm lint
cd frontend && pnpm test -- accessibility
```

eslint-config-next includes jsx-a11y rules and catches missing alt text,
invalid ARIA attributes, and non-interactive elements with handlers. The
accessibility test suite queries by role and accessible name to cover status
announcements, polling dedup, form error associations, and focus preservation.

Manual passes worth doing before shipping a flow:

1. Unplug the mouse. Complete the flow with the keyboard alone.
2. Run VoiceOver (`Cmd+F5` on macOS) through pool details and the prediction
   flow — every control should announce a name, a role, and its state. Connect a
   wallet and submit a prediction to hear the connection, submission, and
confirmation announcements.
3. Run Lighthouse or axe DevTools on `/`, `/dashboard`, and a pool detail page.
4. Zoom to 200% and check nothing is clipped or overlapping.

Automated tooling catches roughly a third of WCAG issues; the keyboard and
screen-reader passes are what find the rest.

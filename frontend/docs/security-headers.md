# Content Security Policy and response headers

`middleware.ts` generates a fresh 128-bit random nonce for each document/RSC
request. It overwrites incoming CSP/nonce headers, forwards the trusted policy
to Next.js, and sends the same enforced policy to the browser. The root layout
renders dynamically and applies the nonce to its inline critical style.
Next.js applies it to framework scripts. This follows the
[Next.js nonce integration](https://nextjs.org/docs/15/app/guides/content-security-policy).

## Production policy

- Scripts require a nonce with `strict-dynamic`; no `unsafe-inline`,
  `unsafe-eval`, external script host wildcard, or inline event handler allowance.
- Stylesheets are same-origin; inline style elements require a nonce. Only
  **style attributes** allow `unsafe-inline`, for React/Radix/Recharts sizing and
  positioning. This does not authorize scripts or event handlers.
- Images allow same-origin, `data:` and `blob:` for local previews; fonts are
  self-hosted by `next/font`. Generated market PNGs remain under `/api/og/pool/`.
- Connections allow the exact origin of `NEXT_PUBLIC_API_BASE_URL`, its derived
  WebSocket origin, and `NEXT_PUBLIC_STELLAR_HORIZON_URL`. Paths and query strings
  are removed; non-HTTP(S), credential-bearing, and wildcard URLs are rejected.
- Framing, embedded objects and base-URL changes are blocked. Forms are
  same-origin. Insecure requests are upgraded to HTTPS.

The current wallet integration uses the browser's Freighter API; it does not
load a remote script or embed a wallet frame. `Cross-Origin-Opener-Policy` is
`same-origin-allow-popups`, and COEP is intentionally not enabled so wallet
popups are not isolated. A new iframe-based wallet needs a reviewed, explicit
policy change rather than a wildcard exception.

`next.config.ts` applies `nosniff`, `X-Frame-Options: DENY`,
`strict-origin-when-cross-origin`, a permissions policy disabling camera,
microphone and geolocation, and the popup-compatible opener policy to all
responses, including API/static responses. Production additionally sends HSTS
with a one-year lifetime (without imposing it on all subdomains).
It also sets a restrictive fallback CSP (`default-src 'none'`) for responses
outside document middleware. Normal document responses replace it with the nonce
policy; a skipped middleware path cannot silently serve unrestricted HTML.

## Development and deployment

Development alone permits script evaluation, inline style elements and local
HMR WebSockets. HSTS and insecure-request upgrading are absent in development.

Set the existing public endpoint variables **before building** and keep them
consistent when starting the server:

| Variable                          | Existing default                      | Production expectation                                                |
| --------------------------------- | ------------------------------------- | --------------------------------------------------------------------- |
| `NEXT_PUBLIC_API_BASE_URL`        | `http://localhost:3000`               | Absolute HTTPS backend URL; its WSS endpoint is allowed automatically |
| `NEXT_PUBLIC_STELLAR_HORIZON_URL` | `https://horizon-testnet.stellar.org` | HTTPS Horizon URL matching the selected Stellar network               |

No new environment variable is required. Defaults match the existing clients;
the localhost API default is for local development, not a deployed backend.
Do not add arbitrary origins to solve a CORS or endpoint configuration error.

Nonce-bearing pages are rendered on each request, with `private, no-store`.
**HTML/RSC must not be cached by a reverse proxy/CDN.** This intentionally
disables static page generation/ISR and increases server rendering work.
Immutable Next assets and the image optimizer retain their existing caching
and SVG-specific CSP. API and public image/font responses do not need document
nonces. CSS optimization through Critters is disabled because its inline
`onload` handlers conflict with `script-src-attr 'none'`.

## Verification

```sh
pnpm test -- --runInBand securityHeaders
pnpm build
pnpm start
# In another terminal, against the production server:
SECURITY_TEST_URL=http://localhost:3000 node --test scripts/security-headers.integration.mjs
```

The HTTP integration checks verify nonce freshness and HTML/header agreement,
spoofed-header replacement, production script restrictions, error-page headers,
immutable asset caching and generated PNG previews. Run against production,
not `next dev`.
The configured backend must contain pool `1`, or set `SECURITY_TEST_POOL_ID`
to an existing pool ID. The preview check compares it with the missing-pool PNG
so a silently failing backend cannot pass by returning only fallback images.

The [browser smoke screenshot](security-headers-smoke.png) shows the production
dashboard using fixture market data, with its chart and API-fed pool list loaded.

Browser smoke checks: load the homepage and a dashboard route; verify hydration
and client navigation, connect/reject/disconnect with the supported wallet,
exercise API and WebSocket connections, and load a generated market preview.
Watch the console for CSP violations. A mocked wallet can validate the current
browser API integration but cannot prove a real extension's signing flow.

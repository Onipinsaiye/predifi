type PolicyOptions = {
  production: boolean;
  apiUrl?: string;
  horizonUrl?: string;
};

function httpOrigin(value: string): URL {
  const url = new URL(value);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    !/^[a-z0-9.:[\]-]+$/i.test(url.hostname)
  ) {
    throw new Error("CSP endpoints must be absolute HTTP(S) URLs without credentials or wildcards");
  }
  return url;
}

/** Allow only the API and Horizon origins used by the existing clients. */
export function contentSecurityPolicy(nonce: string, options: PolicyOptions): string {
  const api = httpOrigin(options.apiUrl ?? "http://localhost:3000");
  const websocket = new URL(api.origin);
  websocket.protocol = api.protocol === "https:" ? "wss:" : "ws:";
  const horizon = httpOrigin(options.horizonUrl ?? "https://horizon-testnet.stellar.org");
  const connections = new Set(["'self'", api.origin, websocket.origin, horizon.origin]);
  if (!options.production) {
    // Next's dev server uses a same-host WebSocket for Fast Refresh.
    connections.add("ws://localhost:*");
    connections.add("ws://127.0.0.1:*");
  }

  return [
    "default-src 'self'",
    `script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${options.production ? "" : " 'unsafe-eval'"}`,
    "script-src-attr 'none'",
    // Dev style-loader needs inline style elements. Production styles use nonces.
    options.production ? `style-src 'self' 'nonce-${nonce}'` : "style-src 'self' 'unsafe-inline'",
    // React/Radix/Recharts use style attributes for positioning and chart dimensions.
    "style-src-attr 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self'",
    `connect-src ${[...connections].join(" ")}`,
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'self'",
    "frame-src 'none'",
    "frame-ancestors 'none'",
    ...(options.production ? ["upgrade-insecure-requests"] : []),
  ].join("; ");
}

/** Applied to documents, API responses and static assets by next.config.ts. */
export function securityHeaders(production: boolean) {
  return [
    // Fail closed if a response bypasses document middleware. Middleware replaces
    // this with the nonce policy; static/API resources do not need active content.
    {
      key: "Content-Security-Policy",
      value: "default-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    },
    { key: "X-Content-Type-Options", value: "nosniff" },
    { key: "X-Frame-Options", value: "DENY" },
    { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
    { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
    // Preserve wallet popup communication; do not enable COEP isolation.
    { key: "Cross-Origin-Opener-Policy", value: "same-origin-allow-popups" },
    ...(production ? [{ key: "Strict-Transport-Security", value: "max-age=31536000" }] : []),
  ];
}

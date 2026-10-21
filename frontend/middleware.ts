import { NextRequest, NextResponse } from "next/server";
import { contentSecurityPolicy } from "./lib/securityHeaders";

export function middleware(request: NextRequest) {
  const nonce = Buffer.from(crypto.getRandomValues(new Uint8Array(16))).toString("base64");
  const policy = contentSecurityPolicy(nonce, {
    production: process.env.NODE_ENV === "production",
    apiUrl: process.env.NEXT_PUBLIC_API_BASE_URL,
    horizonUrl: process.env.NEXT_PUBLIC_STELLAR_HORIZON_URL,
  });
  const headers = new Headers(request.headers);
  // Overwrite untrusted incoming values. Next reads CSP to nonce its own scripts.
  headers.set("x-nonce", nonce);
  headers.set("Content-Security-Policy", policy);
  const response = NextResponse.next({ request: { headers } });
  response.headers.set("Content-Security-Policy", policy);
  // HTML and RSC responses must never reuse another request's nonce via a CDN.
  response.headers.set("Cache-Control", "private, no-store");
  return response;
}

export const config = {
  // Keep image/API caching and immutable asset caching independent of HTML nonces.
  // Prefetch/RSC requests deliberately receive the policy too.
  matcher: [
    "/((?!api/|_next/static/|_next/image|favicon.ico|.*\\.(?:svg|png|jpg|jpeg|webp|ico|woff|woff2|ttf|otf|json|txt|xml)$).*)",
  ],
};

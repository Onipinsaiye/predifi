import type { NextConfig } from "next";
import { securityHeaders } from "./lib/securityHeaders";

const nextConfig: NextConfig = {
  async headers() {
    return [
      {
        source: "/:path*",
        headers: securityHeaders(process.env.NODE_ENV === "production"),
      },
      {
        // Build-time hashed assets — safe to cache forever
        source: "/_next/static/:path*",
        headers: [
          {
            key: "Cache-Control",
            value: "public, max-age=31536000, immutable",
          },
        ],
      },
      {
        // Public folder assets (images, icons, manifests, fonts)
        source: "/(:path*\.(?:ico|png|jpg|jpeg|svg|webp|woff|woff2|ttf|otf|json|txt|xml))",
        headers: [
          {
            key: "Cache-Control",
            value: "public, max-age=86400, must-revalidate",
          },
        ],
      },
    ];
  },
  // Enable CSS minification and optimization in production
  compress: true,
  // Critters' optimizeCss emits inline onload handlers, which strict CSP blocks.
  // Keep Next's normal external stylesheets instead.
  images: {
    // Allow Next.js to optimise SVG files served from the /public directory.
    // The Content-Security-Policy header set below mitigates the XSS risk that
    // comes with serving SVGs as images (they cannot execute scripts when loaded
    // via <img> / next/image).
    dangerouslyAllowSVG: true,
    contentDispositionType: "attachment",
    contentSecurityPolicy: "default-src 'self'; script-src 'none'; sandbox;",
  },
};

export default nextConfig;

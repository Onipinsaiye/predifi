import type { AnchorHTMLAttributes, ReactNode } from "react";

/** Treat untrusted content as plain text; the supported rich-text allowlist is empty. */
export function sanitizeRichText(input: unknown, maxLength = 10_000): string {
  if (typeof input !== "string") return "";
  const withoutDangerousBlocks = input.replace(
    /<(script|style|iframe|object|embed|svg|math)\b[^>]*>[\s\S]*?<\/\1\s*>/gi,
    " ",
  );
  const plainText = withoutDangerousBlocks
    .replace(/<[^>]*>/g, " ")
    .normalize("NFKC")
    .replace(
      /[\u0000-\u001F\u007F-\u009F\u2028\u2029\u200B-\u200F\u202A-\u202E\u2060\u2066-\u2069\uFEFF]/g,
      " ",
    )
    .replace(/\s+/g, " ")
    .trim();
  return plainText.length <= maxLength
    ? plainText
    : `${plainText.slice(0, Math.max(0, maxLength - 1)).trimEnd()}…`;
}

/** Return a normalized web URL, or null for non-web/scriptable destinations. */
export function normalizeExternalUrl(input: unknown): string | null {
  if (typeof input !== "string") return null;
  const candidate = input
    .normalize("NFKC")
    .replace(/[\u0000-\u001F\u007F-\u009F\u200B-\u200F\u202A-\u202E\u2060\u2066-\u2069\uFEFF]/g, "")
    .trim();
  try {
    const url = new URL(candidate);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) return null;
    return url.toString();
  } catch {
    return null;
  }
}

type SafeExternalLinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href" | "children"> & {
  href: string | null | undefined;
  children: ReactNode;
};

export function SafeExternalLink({ href, children, rel, ...props }: SafeExternalLinkProps) {
  const safeHref = normalizeExternalUrl(href);
  return (
    <a
      {...props}
      href={safeHref ?? undefined}
      target="_blank"
      rel={rel ? `${rel} noopener noreferrer` : "noopener noreferrer"}
      aria-disabled={safeHref ? props["aria-disabled"] : "true"}
    >
      {children}
    </a>
  );
}

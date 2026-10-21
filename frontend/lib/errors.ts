export type ErrorKind = "not-found" | "transient" | "unknown";

export interface RouteErrorInfo {
  kind: ErrorKind;
  status: number | null;
  message: string;
  digest: string | null;
  correlationId: string;
  retryable: boolean;
  route: string;
  timestamp: string;
}

export interface RedictedDiagnostics {
  kind: ErrorKind;
  status: number | null;
  digest: string | null;
  correlationId: string;
  retryable: boolean;
  route: string;
  timestamp: string;
  message: string;
}

const SENSITIVE_KEY = /password|token|secret|authorization|cookie|api[:_-]?key|credential|session|sdk|private/i;
const MAX_MESSAGE_LENGTH = 240;

function safeString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function safeStatus(value: unknown): number | null {
  if (typeof value === "number" && Number.isInteger(value) && value >= 100 && value <= 599) {
    return value;
  }
  return null;
}

export function redactSensitive(input: string): string {
  let output = input;
  output = output.replace(
    /([a-z]+?:\/\/[^@\s/]+)@([^\s/]+)/gi,
    "[redacted-credentials]"
  );
  output = output.replace(/(bearer\s+)[^\s,]+/gi, "$1[redacted]");
  output = output.replace(
    /([?&])[\w&=]*(?:password|token|secret|key|auth)[^&=]*=[^&]*/gi,
    "$1[redacted]"
  );
  output = output.replace(
    /([a-z0-9_-]*(?:password|token|secret|authorization|cookie|api[:_-]?key|credential|session|sdk|private)[a-z0-9_-]*)\s*=\s*([^\s,;]+)/gi,
    "$1=[redacted]"
  );
  output = output.replace(
    /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi,
    "[redacted-id]"
  );
  output = output.replace(/\b[A-Za-z0-9_\-.]{32,}\b/g, "[redacted-token]");
  return output;
}

export function sanitizeMessage(message: unknown): string {
  const raw = safeString(message);
  if (!raw) return "Unknown error";
  const redacted = redactSensitive(raw).replace(/\s+/g, " ").trim();
  if (redacted.length <= MAX_MESSAGE_LENGTH) return redacted;
  return `${redacted.slice(0, MAX_MESSAGE_LENGTH - 1)}…`;
}

export function classifyError(error: unknown): ErrorKind {
  const candidate = error as { digest?: unknown; status?: unknown; code?: unknown; name?: unknown } | null;
  const status = safeStatus(candidate?.status);
  if (status === 404) return "not-found";
  if (status !== null && status >= 500) return "transient";
  if (status !== null && status >= 400) return "unknown";
  const code = safeString(candidate?.code);
  if (code && /NOT_FOUND|NOTFOUND|404/i.test(code)) return "not-found";
  const digest = safeString(candidate?.digest);
  if (digest && /NEXT_NOT_FOUND/i.test(digest)) return "not-found";
  const name = safeString(candidate?.name);
  if (name && /NotFound|NOT_FOUND/i.test(name)) return "not-found";
  if (
    name &&
    /(NetworkError|TimeoutError|AbortError|FetchError|ChunkLoadError)/i.test(name)
  ) {
    return "transient";
  }
  if (digest && /CHUNK_LOAD_ERROR|CONNECTION_ERROR|TIMEOUT|ETIMEDOUT/i.test(digest)) {
    return "transient";
  }
  return "unknown";
}

export function isRetryable(kind: ErrorKind): boolean {
  return kind !== "not-found";
}

export function createCorrelationId(): string {
  const cryptoObj = globalThis.crypto as Crypto | undefined;
  if (cryptoObj && typeof cryptoObj.randomUUID === "function") {
    return cryptoObj.randomUUID();
  }
  const bytes = new Uint8Array(16);
  if (cryptoObj && typeof cryptoObj.getRandomValues === "function") {
    cryptoObj.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20, 32)}`;
}

export function normalizeRoute(route: unknown): string {
  const raw = safeString(route);
  if (!raw) return "/";
  const withoutOrigin = raw.replace(/^[a-z][a-z0-9+.-]*:\/\/[^\/]+/i, "");
  const withoutHash = withoutOrigin.split("#")[0];
  const withoutQuery = withoutHash.split("?")[0];
  const normalized = withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
  return normalized.length > 1 ? normalized.replace(/\/+$/, "") : normalized;
}

export function describeRouteError(
  error: unknown,
  options: { route?: unknown; correlationId?: string; timestamp?: string } = {}
): RouteErrorInfo {
  const candidate = error as { digest?: unknown; status?: unknown } | null;
  const kind = classifyError(error);
  const status = safeStatus(candidate?.status);
  const digest = safeString(candidate?.digest);
  const correlationId =
    safeString(options.correlationId) ?? createCorrelationId();
  const timestamp = safeString(options.timestamp) ?? new Date().toISOString();
  return {
    kind,
    status,
    message: sanitizeMessage(error),
    digest,
    correlationId,
    retryable: isRetryable(kind),
    route: normalizeRoute(options.route),
    timestamp,
  };
}

export function redactDiagnostics(info: RouteErrorInfo): RedictedDiagnostics {
  return {
    kind: info.kind,
    status: info.status,
    digest: info.digest ? redactSensitive(info.digest) : null,
    correlationId: redactSensitive(info.correlationId),
    retryable: info.retryable,
    route: redactSensitive(info.route),
    timestamp: info.timestamp,
    message: sanitizeMessage(info.message),
  };
}

export function buildRetryHref(info: RouteErrorInfo, attempt = 0): string {
  const params = new URLSearchParams();
  if (info.route !== "/") params.set("retryFrom", info.route);
  if (attempt > 0) params.set("retryAttempt", String(attempt));
  params.set("correlationId", info.correlationId);
  const query = params.toString();
  return query ? `${info.route}?${query}` : info.route;
}

export function describeRouteErrorForDisplay(error: unknown, route?: unknown): {
  info: RouteErrorInfo;
  diagnostics: RedictedDiagnostics;
  title: string;
  description: string;
  retryHref: string;
  shouldRetry: boolean;
  yearnForNotFound: boolean;
} {
  const info = describeRouteError(error, { route });
  const diagnostics = redactDiagnostics(info);
  const notFound = info.kind === "not-found";
  const title = notFound ? "Market not found" : "Unable to load market data";
  const description = notFound
    ? "The market or dashboard you requested does not exist or is no longer available."
    : "A temporary error interrupted this view. You can retry without losing your current market context.";
  return {
    info,
    diagnostics,
    title,
    description,
    retryHref: buildRetryHref(info),
    shouldRetry: info.retryable,
    yearnForNotFound: notFound,
  };
}
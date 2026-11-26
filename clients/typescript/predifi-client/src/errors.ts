import { isPlainObject } from './internal.js';

/**
 * Base class for every error raised by the client.
 *
 * `name` always matches the concrete subclass so `error.name` remains useful
 * after transpilation and in logs.
 */
export class PredifiError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = new.target.name;
  }
}

/** A caller supplied value (base URL, pagination options, ...) is invalid. */
export class PredifiValidationError extends PredifiError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super(message);
    this.field = field;
  }
}

/** The request never produced an HTTP response (DNS, TLS, abort, ...). */
export class PredifiNetworkError extends PredifiError {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
  }
}

/** The server returned a cursor/offset sequence that cannot make progress. */
export class PredifiPaginationError extends PredifiError {
  readonly cursor?: string;

  constructor(message: string, cursor?: string) {
    super(message);
    this.cursor = cursor;
  }
}

export interface ApiErrorInit {
  status?: number;
  code?: string;
  requestId?: string;
  retryable?: boolean;
  details?: unknown;
  body?: unknown;
  rawBody?: string;
  headers?: Headers;
}

/**
 * A non-success HTTP response, or a success envelope carrying an `error`.
 *
 * The original parsed body, raw text, headers, and request identifier are kept
 * so callers can log or re-surface the server's response without losing
 * detail.
 */
export class PredifiApiError extends PredifiError {
  readonly status?: number;
  readonly code?: string;
  readonly requestId?: string;
  readonly retryable?: boolean;
  readonly details?: unknown;
  readonly body?: unknown;
  readonly rawBody?: string;
  readonly headers?: Headers;

  constructor(message: string, init: ApiErrorInit = {}) {
    super(message);
    this.status = init.status;
    this.code = init.code;
    this.requestId = init.requestId;
    this.retryable = init.retryable;
    this.details = init.details;
    this.body = init.body;
    this.rawBody = init.rawBody;
    this.headers = init.headers;
  }

  static fromResponse(response: Response, parsed: unknown, rawBody: string): PredifiApiError {
    const info = extractApiError(parsed, rawBody, response.status);
    return new PredifiApiError(info.message, {
      ...info,
      status: response.status,
      body: parsed ?? rawBody,
      rawBody,
      headers: response.headers,
    });
  }

  static fromEnvelope(envelope: unknown, status?: number): PredifiApiError {
    const info = extractApiError(envelope, '', status ?? 200);
    return new PredifiApiError(info.message, { ...info, status, body: envelope });
  }
}

export interface VersionErrorInit extends ApiErrorInit {
  requested?: string;
  supported?: string[];
}

/** The server rejected or has retired the negotiated API version. */
export class PredifiVersionError extends PredifiApiError {
  readonly requested?: string;
  readonly supported?: string[];

  constructor(message: string, init: VersionErrorInit = {}) {
    super(message, init);
    this.requested = init.requested;
    this.supported = init.supported;
  }
}

export interface ExtractedApiError {
  message: string;
  code?: string;
  requestId?: string;
  retryable?: boolean;
  details?: unknown;
}

/**
 * Normalise the error shapes the backend publishes: a top-level `error`
 * object, a bare error object, a JSON string, or a non-JSON body.
 */
export function extractApiError(payload: unknown, rawBody: string, status: number): ExtractedApiError {
  if (typeof payload === 'string' && payload.trim() !== '') {
    return { message: payload };
  }

  if (isPlainObject(payload)) {
    const source = isPlainObject(payload.error) ? payload.error : payload;
    const message =
      typeof source.message === 'string'
        ? source.message
        : typeof payload.message === 'string'
          ? payload.message
          : rawBody.trim() !== ''
            ? rawBody
            : `HTTP ${status}`;

    return {
      message,
      code: typeof source.code === 'string' ? source.code : undefined,
      requestId:
        typeof source.request_id === 'string'
          ? source.request_id
          : typeof source.requestId === 'string'
            ? source.requestId
            : undefined,
      retryable: typeof source.retryable === 'boolean' ? source.retryable : undefined,
      details: source.details,
    };
  }

  return { message: rawBody.trim() !== '' ? rawBody : `HTTP ${status}` };
}

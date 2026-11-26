import {
  PredifiApiError,
  PredifiNetworkError,
  PredifiValidationError,
  PredifiVersionError,
  extractApiError,
} from './errors.js';
import { isErrorEnvelope, isUnsupportedVersion, parseJson, unwrapEnvelope } from './internal.js';

export interface PredifiHttpOptions {
  /** Absolute HTTP(S) base URL, optionally including a path prefix. */
  baseUrl: string;
  /** Value sent as `Accept-Version`; 406 responses raise `PredifiVersionError`. */
  apiVersion?: string;
  /** Injectable fetch implementation, primarily for tests and non-standard runtimes. */
  fetch?: typeof fetch;
  /** Headers applied to every request. */
  headers?: Record<string, string>;
  /** Abort requests that take longer than this many milliseconds. */
  timeoutMs?: number;
}

export interface RequestOptions {
  query?: Record<string, unknown>;
  method?: string;
  body?: unknown;
  headers?: HeadersInit;
  signal?: AbortSignal;
  /** Prefix the path with `/api/v1`. Defaults to `true`. */
  apiPrefix?: boolean;
}

/** Low-level HTTP transport shared by every typed resource. */
export class PredifiHttp {
  readonly baseUrl: URL;
  readonly apiVersion?: string;
  /** Version reported by the most recent response, when the server sends one. */
  lastApiVersion?: string;

  private readonly fetchImpl: typeof fetch;
  private readonly defaultHeaders: Headers;
  private readonly timeoutMs?: number;

  constructor(options: PredifiHttpOptions) {
    this.baseUrl = normalizeBaseUrl(options.baseUrl);
    this.apiVersion = options.apiVersion;
    this.fetchImpl = options.fetch ?? globalThis.fetch;
    if (typeof this.fetchImpl !== 'function') {
      throw new PredifiValidationError('a fetch implementation is required', 'fetch');
    }
    this.defaultHeaders = new Headers(options.headers);
    this.timeoutMs = options.timeoutMs;
  }

  resolveUrl(path: string, apiPrefix = true): string {
    const cleanPath = path.replace(/^\/+/, '');
    const basePath = this.baseUrl.pathname.replace(/\/+$/, '');
    const prefix = apiPrefix ? `${basePath}/api/v1/` : `${basePath}/`;
    const url = new URL(this.baseUrl.toString());
    url.pathname = `${prefix}${cleanPath}`;
    url.search = '';
    url.hash = '';
    return url.toString();
  }

  async request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const { query, method = 'GET', body, headers, signal, apiPrefix = true } = options;
    const url = `${this.resolveUrl(path, apiPrefix)}${buildQuery(query)}`;

    const mergedHeaders = new Headers(this.defaultHeaders);
    if (this.apiVersion) {
      mergedHeaders.set('accept-version', this.apiVersion);
    }
    if (headers) {
      new Headers(headers).forEach((value, key) => mergedHeaders.set(key, value));
    }

    let payload: BodyInit | undefined;
    if (body !== undefined) {
      if (!mergedHeaders.has('content-type')) {
        mergedHeaders.set('content-type', 'application/json');
      }
      payload = typeof body === 'string' ? body : JSON.stringify(body);
    }

    const timeoutSignal = this.timeoutMs ? AbortSignal.timeout(this.timeoutMs) : undefined;

    let response: Response;
    try {
      response = await this.fetchImpl(url, {
        method,
        headers: mergedHeaders,
        body: payload,
        signal: signal ?? timeoutSignal,
      });
    } catch (cause) {
      throw new PredifiNetworkError(`request to ${url} failed`, { cause });
    }

    const responseVersion =
      response.headers.get('x-api-version') ?? response.headers.get('x-predifi-api-version');
    if (responseVersion) {
      this.lastApiVersion = responseVersion;
    }

    const rawBody = await response.text();
    const parsed = parseJson(rawBody);

    if (!response.ok) {
      if (response.status === 406 || isUnsupportedVersion(parsed)) {
        throw new PredifiVersionError(
          `API version ${this.apiVersion ?? '(unspecified)'} is not supported by ${this.baseUrl.origin}`,
          {
            status: response.status,
            requested: this.apiVersion,
            body: parsed ?? rawBody,
            rawBody,
            headers: response.headers,
          },
        );
      }
      throw PredifiApiError.fromResponse(response, parsed, rawBody);
    }

    if (isErrorEnvelope(parsed)) {
      const info = extractApiError(parsed, rawBody, response.status);
      throw new PredifiApiError(info.message, {
        ...info,
        status: response.status,
        body: parsed,
        rawBody,
        headers: response.headers,
      });
    }

    return unwrapEnvelope(parsed) as T;
  }

  get<T>(path: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>(path, { ...options, method: 'GET' });
  }
}

export function normalizeBaseUrl(input: string): URL {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    throw new PredifiValidationError(`invalid base URL: ${input}`, 'baseUrl');
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new PredifiValidationError('baseUrl must use http or https', 'baseUrl');
  }
  if (url.search || url.hash) {
    throw new PredifiValidationError('baseUrl must not include a query or fragment', 'baseUrl');
  }
  url.pathname = url.pathname.replace(/\/+$/, '');
  return url;
}

export function buildQuery(query?: Record<string, unknown>): string {
  if (!query) {
    return '';
  }

  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null) {
      continue;
    }
    if (Array.isArray(value)) {
      if (value.length > 0) {
        params.set(key, value.join(','));
      }
    } else if (typeof value === 'boolean') {
      params.set(key, value ? 'true' : 'false');
    } else {
      params.set(key, String(value));
    }
  }

  const serialized = params.toString();
  return serialized ? `?${serialized}` : '';
}

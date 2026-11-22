const FALLBACK_PREFIX = "predifi:";
const DEFAULT_MAX_AGE_SECONDS = 60 * 60 * 24 * 365;

/**
 * Storage key for the read-only demo-mode flag. Demo state is intentionally
 * unscoped (not tied to a network or wallet) so it survives reloads and can be
 * cleared in one place when the user exits to live data.
 */
export const DEMO_MODE_STORAGE_KEY = "demo-mode";
const DEMO_MODE_SCOPE = "demo";
const DEMO_MODE_VALUE = "1";

function canUseBrowserStorage(): boolean {
  return typeof window !== "undefined" && typeof document !== "undefined";
}

function fallbackKey(key: string): string {
  return `${FALLBACK_PREFIX}${key}`;
}

/**
 * Builds a namespaced storage key from an optional identity scope
 * (e.g. network + address). Scoped values never collide with unscoped ones,
 * so account-specific data can be cleared on disconnect/account change.
 */
function scopedKey(key: string, scope?: string | null): string {
  return scope ? `${scope}:${key}` : key;
}

function readCookie(key: string): string | null {
  try {
    const encodedKey = `${encodeURIComponent(key)}=`;
    const cookie = document.cookie
      .split("; ")
      .find((entry) => entry.startsWith(encodedKey));

    return cookie ? decodeURIComponent(cookie.slice(encodedKey.length)) : null;
  } catch {
    return null;
  }
}

function writeCookie(
  key: string,
  value: string,
  maxAgeSeconds: number,
): boolean {
  const secure = window.location.protocol === "https:" ? "; Secure" : "";

  try {
    document.cookie = `${encodeURIComponent(key)}=${encodeURIComponent(value)}; Path=/; Max-Age=${maxAgeSeconds}; SameSite=Lax${secure}`;
    return readCookie(key) === value;
  } catch {
    return false;
  }
}

function readLocalStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(fallbackKey(key));
  } catch {
    return null;
  }
}

/**
 * Reads a value from cookies first, then localStorage. The fallback keeps
 * preferences available in browsers or privacy modes that block cookies.
 */
export function getPersistedValue(
  key: string,
  scope?: string | null,
): string | null {
  if (!canUseBrowserStorage()) return null;

  const namespacedKey = scopedKey(key, scope);
  return readCookie(namespacedKey) ?? readLocalStorage(namespacedKey);
}

/**
 * Persists a value in a cookie and falls back to localStorage when the browser
 * silently rejects or blocks the cookie write.
 */
export function setPersistedValue(
  key: string,
  value: string,
  maxAgeSeconds = DEFAULT_MAX_AGE_SECONDS,
  scope?: string | null,
): boolean {
  if (!canUseBrowserStorage()) return false;

  const namespacedKey = scopedKey(key, scope);

  if (writeCookie(namespacedKey, value, maxAgeSeconds)) {
    try {
      window.localStorage.removeItem(fallbackKey(namespacedKey));
    } catch {
      // The cookie write succeeded, so an unavailable localStorage is harmless.
    }
    return true;
  }

  try {
    window.localStorage.setItem(fallbackKey(namespacedKey), value);
    return true;
  } catch {
    return false;
  }
}

export function removePersistedValue(key: string, scope?: string | null): void {
  if (!canUseBrowserStorage()) return;

  const namespacedKey = scopedKey(key, scope);

  writeCookie(namespacedKey, "", 0);

  try {
    window.localStorage.removeItem(fallbackKey(namespacedKey));
  } catch {
    // Storage can be unavailable in strict privacy contexts.
  }
}

/**
 * Returns true when the user has opted into the read-only demo mode. The flag
 * is stored under a dedicated demo scope so it never collides with live,
 * network- or account-scoped preferences.
 */
export function isDemoModePersisted(): boolean {
  return (
    getPersistedValue(DEMO_MODE_STORAGE_KEY, DEMO_MODE_SCOPE) ===
    DEMO_MODE_VALUE
  );
}

/**
 * Persists the demo-mode flag so the read-only state survives reloads.
 * Returns true when the flag was stored successfully.
 */
export function persistDemoMode(): boolean {
  return setPersistedValue(
    DEMO_MODE_STORAGE_KEY,
    DEMO_MODE_VALUE,
    DEFAULT_MAX_AGE_SECONDS,
    DEMO_MODE_SCOPE,
  );
}

/**
 * Clears the demo-mode flag. Called when exiting to live data so no demo
 * state lingers in cookies or localStorage.
 */
export function clearDemoMode(): void {
  removePersistedValue(DEMO_MODE_STORAGE_KEY, DEMO_MODE_SCOPE);
}

/**
 * Storage key for the user's saved personal market-list views. Entries are
 * scoped by wallet identity so views never leak between accounts.
 */
export const SAVED_VIEWS_STORAGE_KEY = "saved-market-views";

/**
 * Schema version for the persisted saved-views payload. Bump this whenever the
 * stored shape changes so older payloads can be migrated or discarded.
 */
export const SAVED_VIEWS_SCHEMA_VERSION = 1;

/** Upper bound on the number of named views a single wallet may store. */
export const MAX_SAVED_VIEWS = 20;

/** Upper bound on the length of a view name. */
export const MAX_VIEW_NAME_LENGTH = 60;

const SAVED_VIEWS_SCOPE_PREFIX = "views";

export type MarketSortDirection = "asc" | "desc";

export interface MarketViewFilters {
  search: string;
  category: string | null;
  status: string | null;
  sortBy: string;
  sortDirection: MarketSortDirection;
}

export interface SavedMarketView {
  id: string;
  name: string;
  filters: MarketViewFilters;
  createdAt: number;
}

interface SavedViewsPayload {
  version: number;
  views: SavedMarketView[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sanitizeString(value: unknown, maxLength: number): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > maxLength) return null;
  return trimmed;
}

function sanitizeNullableString(
  value: unknown,
  maxLength: number,
): string | null {
  if (value === null || value === undefined) return null;
  return sanitizeString(value, maxLength);
}

/**
 * Validates and normalizes an untrusted filter/sort state. Returns null when
 * the state is unusable so callers can discard it instead of persisting junk.
 */
export function validateMarketViewFilters(
  value: unknown,
): MarketViewFilters | null {
  if (!isRecord(value)) return null;

  const search = sanitizeNullableString(value.search, 200) ?? "";
  const category = sanitizeNullableString(value.category, 80);
  const status = sanitizeNullableString(value.status, 80);
  const sortBy = sanitizeString(value.sortBy, 80);
  const sortDirection = value.sortDirection;

  if (!sortBy) return null;
  if (sortDirection !== "asc" && sortDirection !== "desc") return null;

  return { search, category, status, sortBy, sortDirection };
}

function validateSavedView(value: unknown): SavedMarketView | null {
  if (!isRecord(value)) return null;

  const id = sanitizeString(value.id, 80);
  const name = sanitizeString(value.name, MAX_VIEW_NAME_LENGTH);
  const filters = validateMarketViewFilters(value.filters);
  const createdAt =
    typeof value.createdAt === "number" && Number.isFinite(value.createdAt)
      ? value.createdAt
      : Date.now();

  if (!id || !name || !filters) return null;

  return { id, name, filters, createdAt };
}

/**
 * Parses a raw persisted payload into a validated list of views. Corrupted or
 * incompatible payloads yield an empty list so the caller can recover safely.
 */
export function parseSavedViewsPayload(raw: string | null): SavedMarketView[] {
  if (!raw) return [];

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }

  if (!isRecord(parsed)) return [];

  // Unknown/newer schema versions are discarded rather than guessed at.
  if (parsed.version !== SAVED_VIEWS_SCHEMA_VERSION) return [];
  if (!Array.isArray(parsed.views)) return [];

  const seenIds = new Set<string>();
  const seenNames = new Set<string>();
  const views: SavedMarketView[] = [];

  for (const entry of parsed.views) {
    const view = validateSavedView(entry);
    if (!view) continue;

    const nameKey = view.name.toLowerCase();
    if (seenIds.has(view.id) || seenNames.has(nameKey)) continue;

    seenIds.add(view.id);
    seenNames.add(nameKey);
    views.push(view);

    if (views.length >= MAX_SAVED_VIEWS) break;
  }

  return views;
}

export function serializeSavedViews(views: SavedMarketView[]): string {
  const payload: SavedViewsPayload = {
    version: SAVED_VIEWS_SCHEMA_VERSION,
    views: views.slice(0, MAX_SAVED_VIEWS),
  };
  return JSON.stringify(payload);
}

function savedViewsScope(walletAddress: string): string {
  return `${SAVED_VIEWS_SCOPE_PREFIX}:${walletAddress.toLowerCase()}`;
}

/**
 * Loads the saved views for a wallet. Corrupted payloads are cleared from
 * storage and reported as an empty list.
 */
export function loadSavedViews(walletAddress: string | null): SavedMarketView[] {
  if (!walletAddress) return [];

  const raw = getPersistedValue(
    SAVED_VIEWS_STORAGE_KEY,
    savedViewsScope(walletAddress),
  );
  if (raw === null) return [];

  const views = parseSavedViewsPayload(raw);
  if (views.length === 0) {
    // Either empty or corrupted; drop the bad payload so it cannot resurface.
    removePersistedValue(
      SAVED_VIEWS_STORAGE_KEY,
      savedViewsScope(walletAddress),
    );
  }
  return views;
}

/**
 * Persists the saved views for a wallet. Returns the stored list, which may be
 * truncated to MAX_SAVED_VIEWS, or null when storage is unavailable.
 */
export function saveSavedViews(
  walletAddress: string | null,
  views: SavedMarketView[],
): SavedMarketView[] | null {
  if (!walletAddress) return null;

  const bounded = views.slice(0, MAX_SAVED_VIEWS);
  const ok = setPersistedValue(
    SAVED_VIEWS_STORAGE_KEY,
    serializeSavedViews(bounded),
    DEFAULT_MAX_AGE_SECONDS,
    savedViewsScope(walletAddress),
  );

  return ok ? bounded : null;
}

/**
 * Adds or replaces a named view for a wallet. Names are unique per wallet
 * (case-insensitive) and the list is bounded to MAX_SAVED_VIEWS entries.
 */
export function upsertSavedView(
  walletAddress: string | null,
  view: SavedMarketView,
): SavedMarketView[] | null {
  if (!walletAddress) return null;

  const validated = validateSavedView(view);
  if (!validated) return null;

  const existing = loadSavedViews(walletAddress);
  const nameKey = validated.name.toLowerCase();
  const withoutDuplicate = existing.filter(
    (entry) =>
      entry.id !== validated.id && entry.name.toLowerCase() !== nameKey,
  );

  return saveSavedViews(walletAddress, [validated, ...withoutDuplicate]);
}

export function removeSavedView(
  walletAddress: string | null,
  viewId: string,
): SavedMarketView[] | null {
  if (!walletAddress) return null;

  const existing = loadSavedViews(walletAddress);
  return saveSavedViews(
    walletAddress,
    existing.filter((entry) => entry.id !== viewId),
  );
}

/**
 * Clears all saved views for a wallet. Called on wallet disconnect so a
 * reconnect starts from a clean, isolated state.
 */
export function clearSavedViews(walletAddress: string | null): void {
  if (!walletAddress) return;
  removePersistedValue(
    SAVED_VIEWS_STORAGE_KEY,
    savedViewsScope(walletAddress),
  );
}

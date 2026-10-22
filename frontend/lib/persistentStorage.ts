const FALLBACK_PREFIX = "predifi:";
const DEFAULT_MAX_AGE_SECONDS = 60 * 60 * 24 * 365;

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

"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter, usePathname, useSearchParams } from "next/navigation";
import type { PoolsQuery } from "@/lib/api/pools";
import { useDebounce } from "@/lib/hooks/useDebounce";
import { useWallet } from "@/lib/hooks/useWallet";

/**
 * Valid sort options exposed in the UI.
 * Mirrors the `sort_by` field of {@link PoolsQuery}.
 */
export type SortBy = NonNullable<PoolsQuery["sort_by"]>;

/**
 * Valid lifecycle status options.
 * Mirrors the `status` field of {@link PoolsQuery}.
 */
export type PoolStatus = NonNullable<PoolsQuery["status"]>;

/** The shape of the query state managed by {@link usePoolsQuery}. */
export interface PoolsQueryState {
  /** Raw (un-debounced) search string entered by the user. */
  search: string;
  /** Debounced search string — safe to pass to {@link usePools}. */
  debouncedSearch: string;
  /** Current sort order. */
  sortBy: SortBy;
  /** Current lifecycle status filter. */
  status: PoolStatus;
  /** Current category filter (empty string = all categories). */
  category: string;
  /** Current page index (0-based). */
  page: number;
  /** Number of items per page. */
  pageSize: number;
}

/** Actions returned alongside the state to update it. */
export interface PoolsQueryActions {
  setSearch: (value: string) => void;
  setSortBy: (value: SortBy) => void;
  setStatus: (value: PoolStatus) => void;
  setCategory: (value: string) => void;
  /** Navigate to a specific page (0-based). */
  setPage: (page: number) => void;
  setPageSize: (size: number) => void;
  /** Reset all filters back to their defaults. */
  reset: () => void;
  /**
   * The {@link PoolsQuery} object ready to be passed to {@link usePools}.
   * Uses the debounced search to avoid a request on every keystroke.
   */
  poolsQuery: PoolsQuery;
  /**
   * Identity-scoped SWR cache key (`["pools", identityKey, query]`). Pass this
   * to `usePools` so cached data is never shared across network/account.
   */
  poolsQueryKey: readonly ["pools", string, PoolsQuery];
}

/** Default configuration values for the query state. */
export interface UsePoolsQueryOptions {
  defaultSortBy?: SortBy;
  defaultStatus?: PoolStatus;
  defaultCategory?: string;
  defaultPageSize?: number;
  /** Debounce delay for the search field in milliseconds. Default: 350 ms. */
  searchDebounceMs?: number;
  /** Whether to sync query state to the URL as search params. Default: true. */
  syncUrl?: boolean;
  /**
   * Identity key (network + address) used to scope the query. When it changes,
   * pagination and filters are reset so no prior identity's data is reused.
   * Defaults to the connected wallet's network/address identity.
   */
  identityKey?: string;
}

const DEFAULT_SORT_BY: SortBy = "new";
const DEFAULT_STATUS: PoolStatus = "active";
const DEFAULT_PAGE_SIZE = 20;

const VALID_SORT_OPTIONS: readonly SortBy[] = ["new", "popular", "ending_soon"];
const VALID_STATUS_OPTIONS: readonly PoolStatus[] = ["active", "closed", "settled"];

// ---------------------------------------------------------------------------
// Saved market views
// ---------------------------------------------------------------------------

/**
 * Version of the persisted saved-views payload. Bump this whenever the stored
 * shape changes so old entries can be migrated or safely discarded.
 */
export const SAVED_VIEWS_STORAGE_VERSION = 1;

/** Maximum number of saved views a single wallet may keep. */
export const MAX_SAVED_VIEWS = 20;

/** Maximum length of a saved view's name. */
export const MAX_SAVED_VIEW_NAME_LENGTH = 40;

/** The validated filter/sort state captured by a saved view. */
export interface SavedViewFilters {
  search: string;
  sortBy: SortBy;
  status: PoolStatus;
  category: string;
  pageSize: number;
}

/** A single named, private saved view. */
export interface SavedMarketView {
  id: string;
  name: string;
  filters: SavedViewFilters;
  createdAt: number;
  updatedAt: number;
}

/** The versioned payload persisted to storage. */
export interface SavedViewsPayload {
  version: number;
  views: SavedMarketView[];
}

/** Actions returned by {@link useSavedMarketViews}. */
export interface SavedMarketViewsActions {
  /** Save the current filters under a new name. Returns the new view or null. */
  saveView: (name: string, filters: SavedViewFilters) => SavedMarketView | null;
  /** Rename an existing view. Returns true when the rename succeeded. */
  renameView: (id: string, name: string) => boolean;
  /** Delete a saved view. */
  deleteView: (id: string) => void;
  /** Select a view, returning its validated filters (or null if missing). */
  selectView: (id: string) => SavedViewFilters | null;
}

/** Result of {@link useSavedMarketViews}. */
export interface UseSavedMarketViewsResult extends SavedMarketViewsActions {
  views: SavedMarketView[];
  /** True once the persisted state has been read on the client. */
  isLoaded: boolean;
  /** True when the wallet is connected (views are wallet-scoped). */
  isWalletConnected: boolean;
  /** True when the saved-view limit has been reached. */
  isAtCapacity: boolean;
}

/** Validates and normalises a view name. Returns null when invalid. */
export function normalizeViewName(name: unknown): string | null {
  if (typeof name !== "string") return null;
  const trimmed = name.trim();
  if (!trimmed) return null;
  return trimmed.slice(0, MAX_SAVED_VIEW_NAME_LENGTH);
}

/** Validates and normalises a filter/sort state. Returns null when invalid. */
export function validateSavedViewFilters(input: unknown): SavedViewFilters | null {
  if (!input || typeof input !== "object") return null;
  const raw = input as Record<string, unknown>;

  const search = typeof raw.search === "string" ? raw.search.trim().slice(0, 200) : "";
  const sortBy = VALID_SORT_OPTIONS.includes(raw.sortBy as SortBy)
    ? (raw.sortBy as SortBy)
    : DEFAULT_SORT_BY;
  const status = VALID_STATUS_OPTIONS.includes(raw.status as PoolStatus)
    ? (raw.status as PoolStatus)
    : DEFAULT_STATUS;
  const category = typeof raw.category === "string" ? raw.category.trim().slice(0, 100) : "";
  const rawPageSize = Number(raw.pageSize);
  const pageSize =
    Number.isFinite(rawPageSize) && rawPageSize > 0
      ? Math.min(Math.floor(rawPageSize), 100)
      : DEFAULT_PAGE_SIZE;

  return { search, sortBy, status, category, pageSize };
}

/** Validates a single persisted view entry. Returns null when invalid. */
function validateSavedView(input: unknown): SavedMarketView | null {
  if (!input || typeof input !== "object") return null;
  const raw = input as Record<string, unknown>;
  if (typeof raw.id !== "string" || !raw.id) return null;
  const name = normalizeViewName(raw.name);
  if (!name) return null;
  const filters = validateSavedViewFilters(raw.filters);
  if (!filters) return null;
  const createdAt = Number.isFinite(Number(raw.createdAt)) ? Number(raw.createdAt) : Date.now();
  const updatedAt = Number.isFinite(Number(raw.updatedAt)) ? Number(raw.updatedAt) : createdAt;
  return { id: raw.id, name, filters, createdAt, updatedAt };
}

/**
 * Serialises saved views into a versioned payload string.
 * Exported for testing.
 */
export function serializeSavedViews(views: SavedMarketView[]): string {
  const payload: SavedViewsPayload = {
    version: SAVED_VIEWS_STORAGE_VERSION,
    views: views.slice(0, MAX_SAVED_VIEWS),
  };
  return JSON.stringify(payload);
}

/**
 * Parses a persisted payload, recovering safely from corrupted or
 * version-mismatched state. Exported for testing.
 */
export function parseSavedViews(raw: string | null): SavedMarketView[] {
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!parsed || typeof parsed !== "object") return [];
  const payload = parsed as Record<string, unknown>;
  // Discard payloads written by an incompatible version.
  if (payload.version !== SAVED_VIEWS_STORAGE_VERSION) return [];
  if (!Array.isArray(payload.views)) return [];
  const views: SavedMarketView[] = [];
  for (const entry of payload.views) {
    const view = validateSavedView(entry);
    if (view) views.push(view);
    if (views.length >= MAX_SAVED_VIEWS) break;
  }
  return views;
}

/** Builds the wallet-scoped storage key. */
function savedViewsStorageKey(identityKey: string): string {
  return `pools:saved-views:${SAVED_VIEWS_STORAGE_VERSION}:${identityKey}`;
}

/** Generates a reasonably unique id without external dependencies. */
function generateViewId(): string {
  return `view_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * useSavedMarketViews — wallet-scoped, versioned persistence of named private
 * market-list views.
 *
 * Views are stored per wallet identity so disconnecting/reconnecting to a
 * different account never exposes another wallet's saved views. Stored state
 * is versioned and validated on read, so corrupted or outdated payloads are
 * safely discarded rather than crashing the UI.
 *
 * @example
 * const { views, saveView, selectView } = useSavedMarketViews();
 * saveView("Active ETH", { search: "", sortBy: "new", status: "active", category: "eth", pageSize: 20 });
 */
export function useSavedMarketViews(identityKeyOption?: string): UseSavedMarketViewsResult {
  const { network, address } = useWallet();
  const identityKey = identityKeyOption ?? `${network ?? "none"}:${address ?? "none"}`;
  const isWalletConnected = Boolean(address);

  const [views, setViews] = useState<SavedMarketView[]>([]);
  const [isLoaded, setIsLoaded] = useState(false);
  const currentIdentityRef = useRef(identityKey);

  // Load persisted views whenever the identity changes. Switching wallets
  // resets in-memory state so no cross-wallet leakage occurs.
  useEffect(() => {
    currentIdentityRef.current = identityKey;
    setIsLoaded(false);
    if (typeof window === "undefined" || !isWalletConnected) {
      setViews([]);
      setIsLoaded(true);
      return;
    }
    let stored: string | null = null;
    try {
      stored = window.localStorage.getItem(savedViewsStorageKey(identityKey));
    } catch {
      stored = null;
    }
    setViews(parseSavedViews(stored));
    setIsLoaded(true);
  }, [identityKey, isWalletConnected]);

  // Persist on change, but only for the identity currently in memory.
  const persist = useCallback(
    (next: SavedMarketView[]) => {
      if (typeof window === "undefined" || !isWalletConnected) return;
      try {
        window.localStorage.setItem(
          savedViewsStorageKey(currentIdentityRef.current),
          serializeSavedViews(next),
        );
      } catch {
        // Storage may be unavailable (private mode / quota) — fail silently.
      }
    },
    [isWalletConnected],
  );

  const saveView = useCallback(
    (name: string, filters: SavedViewFilters): SavedMarketView | null => {
      const normalizedName = normalizeViewName(name);
      const validatedFilters = validateSavedViewFilters(filters);
      if (!normalizedName || !validatedFilters) return null;
      if (!isWalletConnected) return null;

      let created: SavedMarketView | null = null;
      setViews((prev) => {
        if (prev.length >= MAX_SAVED_VIEWS) return prev;
        const now = Date.now();
        created = {
          id: generateViewId(),
          name: normalizedName,
          filters: validatedFilters,
          createdAt: now,
          updatedAt: now,
        };
        const next = [created, ...prev];
        persist(next);
        return next;
      });
      return created;
    },
    [isWalletConnected, persist],
  );

  const renameView = useCallback(
    (id: string, name: string): boolean => {
      const normalizedName = normalizeViewName(name);
      if (!normalizedName) return false;
      let renamed = false;
      setViews((prev) => {
        const next = prev.map((view) => {
          if (view.id !== id) return view;
          renamed = true;
          return { ...view, name: normalizedName, updatedAt: Date.now() };
        });
        if (renamed) persist(next);
        return renamed ? next : prev;
      });
      return renamed;
    },
    [persist],
  );

  const deleteView = useCallback(
    (id: string) => {
      setViews((prev) => {
        const next = prev.filter((view) => view.id !== id);
        if (next.length === prev.length) return prev;
        persist(next);
        return next;
      });
    },
    [persist],
  );

  const selectView = useCallback(
    (id: string): SavedViewFilters | null => {
      const view = views.find((entry) => entry.id === id);
      if (!view) return null;
      return validateSavedViewFilters(view.filters);
    },
    [views],
  );

  return {
    views,
    isLoaded,
    isWalletConnected,
    isAtCapacity: views.length >= MAX_SAVED_VIEWS,
    saveView,
    renameView,
    deleteView,
    selectView,
  };
}

/**
 * usePoolsQuery — centralised SWR query-parameter state for pool lists.
 *
 * Manages filter/sort/pagination state and (optionally) keeps it in sync with
 * the URL so that searches are bookmarkable and the browser back-button works.
 * Returns both the raw state and the derived {@link PoolsQuery} object ready
 * to hand to {@link usePools}.
 *
 * @example
 * const { poolsQuery, search, setSearch, setSortBy, setPage } = usePoolsQuery();
 * const { pools, total, isLoading } = usePools(poolsQuery);
 */
export function usePoolsQuery(options: UsePoolsQueryOptions = {}): PoolsQueryState & PoolsQueryActions {
  const {
    defaultSortBy = DEFAULT_SORT_BY,
    defaultStatus = DEFAULT_STATUS,
    defaultCategory = "",
    defaultPageSize = DEFAULT_PAGE_SIZE,
    searchDebounceMs = 350,
    syncUrl = true,
    identityKey: identityKeyOption,
  } = options;

  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();

  // ---------------------------------------------------------------------------
  // Identity scoping — pools data is owned per network+address. When the
  // identity changes we reset filters/pagination so the previous identity's
  // query state (and therefore cached SWR data) is never reused.
  // ---------------------------------------------------------------------------
  const { network, address } = useWallet();
  const identityKey = identityKeyOption ?? `${network ?? "none"}:${address ?? "none"}`;

  const previousIdentityRef = useRef(identityKey);
  useEffect(() => {
    if (previousIdentityRef.current === identityKey) return;
    previousIdentityRef.current = identityKey;
    if (syncUrl) {
      router.replace(pathname, { scroll: false });
    }
  }, [identityKey, syncUrl, router, pathname]);

  // ---------------------------------------------------------------------------
  // Read current state from URL (or fall back to defaults)
  // ---------------------------------------------------------------------------
  const rawSearch = syncUrl ? (searchParams.get("q") ?? "") : "";
  const search = rawSearch.trim();

  const rawSort = syncUrl ? (searchParams.get("sort") as SortBy | null) : null;
  const sortBy = rawSort && VALID_SORT_OPTIONS.includes(rawSort) ? rawSort : defaultSortBy;

  const rawStatus = syncUrl ? (searchParams.get("status") as PoolStatus | null) : null;
  const status = rawStatus && VALID_STATUS_OPTIONS.includes(rawStatus) ? rawStatus : defaultStatus;

  const category = syncUrl ? (searchParams.get("category") ?? defaultCategory) : defaultCategory;

  const rawPage = syncUrl ? Number(searchParams.get("page")) : 0;
  const page = !Number.isNaN(rawPage) && rawPage > 0 ? Math.floor(rawPage) : 0;

  const rawPageSize = syncUrl ? Number(searchParams.get("limit")) : defaultPageSize;
  const pageSize = !Number.isNaN(rawPageSize) && rawPageSize > 0 ? Math.floor(rawPageSize) : defaultPageSize;

  // Debounce the raw search value so we don't fire a new SWR request on every
  // keystroke — only after the user pauses for `searchDebounceMs`.
  const debouncedSearch = useDebounce(search, searchDebounceMs);

  // ---------------------------------------------------------------------------
  // URL helper — builds a new URLSearchParams replacing only the changed key
  // ---------------------------------------------------------------------------
  const buildParams = useCallback(
    (overrides: Record<string, string | number | null>): string => {
      const next = new URLSearchParams(searchParams.toString());
      for (const [key, value] of Object.entries(overrides)) {
        if (value === null || value === "" || value === 0) {
          next.delete(key);
        } else {
          next.set(key, String(value));
        }
      }
      return next.toString();
    },
    [searchParams],
  );

  const navigate = useCallback(
    (params: Record<string, string | number | null>) => {
      if (!syncUrl) return;
      const qs = buildParams(params);
      router.replace(`${pathname}${qs ? `?${qs}` : ""}`, { scroll: false });
    },
    [syncUrl, buildParams, router, pathname],
  );

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------
  const setSearch = useCallback(
    (value: string) => navigate({ q: value || null, page: null }),
    [navigate],
  );

  const setSortBy = useCallback(
    (value: SortBy) => navigate({ sort: value === defaultSortBy ? null : value, page: null }),
    [navigate, defaultSortBy],
  );

  const setStatus = useCallback(
    (value: PoolStatus) => navigate({ status: value === defaultStatus ? null : value, page: null }),
    [navigate, defaultStatus],
  );

  const setCategory = useCallback(
    (value: string) => navigate({ category: value || null, page: null }),
    [navigate],
  );

  const setPage = useCallback(
    (p: number) => navigate({ page: p > 0 ? p : null }),
    [navigate],
  );

  const setPageSize = useCallback(
    (size: number) => navigate({ limit: size !== defaultPageSize ? size : null, page: null }),
    [navigate, defaultPageSize],
  );

  const reset = useCallback(() => {
    if (syncUrl) {
      router.replace(pathname, { scroll: false });
    }
  }, [syncUrl, router, pathname]);

  // ---------------------------------------------------------------------------
  // Derived PoolsQuery — uses the *debounced* search to prevent request spam
  // ---------------------------------------------------------------------------
  const poolsQuery = useMemo<PoolsQuery>(() => {
    const q: PoolsQuery = {
      sort_by: sortBy,
      status,
      limit: pageSize,
      offset: page * pageSize,
    };
    if (category) q.category = category;
    // NOTE: The backend `/api/v1/pools` endpoint does not yet support a `q`
    // (text search) filter. We keep `debouncedSearch` in state so callers can
    // use it for client-side filtering until the backend adds full-text search.
    return q;
  }, [sortBy, status, category, page, pageSize]);

  // Cache key scoped by identity so switching network/account never reuses the
  // previous identity's pool data. Callers should pass this to `usePools`.
  const poolsQueryKey = useMemo(
    () => ["pools", identityKey, poolsQuery] as const,
    [identityKey, poolsQuery],
  );

  return {
    // State
    search,
    debouncedSearch,
    sortBy,
    status,
    category,
    page,
    pageSize,
    // Actions
    setSearch,
    setSortBy,
    setStatus,
    setCategory,
    setPage,
    setPageSize,
    reset,
    poolsQuery,
    poolsQueryKey,
  };
}

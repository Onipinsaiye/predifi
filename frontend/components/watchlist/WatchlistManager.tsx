"use client";

/**
 * Watchlist manager component (Issue #280).
 *
 * Renders the authenticated user's watchlists with:
 * - Create new watchlist form.
 * - Per-list edit panel (name, description, public/private toggle with
 *   shareable link display and copy button).
 * - Market list per watchlist (pool IDs; names fetched via /api/v1/pools/:id).
 * - Remove market / delete watchlist actions.
 * - Revocation feedback: once public access is revoked, previously shared
 *   links are marked as invalid.
 */

import { useState } from "react";
import useSWR from "swr";
import { Button, Input } from "@/components/ui";
import { cn } from "@/lib/utils";
import {
  createWatchlist,
  updateWatchlist,
  deleteWatchlist,
  addMarketToWatchlist,
  removeMarketFromWatchlist,
  fetchWatchlists,
  watchlistsUrl,
  publicWatchlistShareUrl,
  type Watchlist,
} from "@/lib/api/watchlist";

// ─── Props ────────────────────────────────────────────────────────────────────

interface WatchlistManagerProps {
  address: string;
  bearerToken: string;
}

// ─── Create form ──────────────────────────────────────────────────────────────

function CreateWatchlistForm({
  address,
  bearerToken,
  onCreated,
}: {
  address: string;
  bearerToken: string;
  onCreated: (wl: Watchlist) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [isPublic, setIsPublic] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [open, setOpen] = useState(false);

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) {
      setError("Watchlist name is required.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const wl = await createWatchlist(address, bearerToken, {
        name: name.trim(),
        description: description.trim() || null,
        public: isPublic,
      });
      onCreated(wl);
      setName("");
      setDescription("");
      setIsPublic(false);
      setOpen(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create watchlist.");
    } finally {
      setSaving(false);
    }
  }

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="rounded-lg border border-zinc-800 px-4 py-2 text-sm text-zinc-400 hover:text-zinc-300 hover:border-zinc-700 transition-colors"
      >
        + New watchlist
      </button>
    );
  }

  return (
    <form
      onSubmit={handleCreate}
      className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-4 space-y-3"
    >
      <Input
        label="Name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="My watchlist"
        autoFocus
      />
      <div className="space-y-1">
        <label className="text-sm font-medium text-zinc-200">Description (optional)</label>
        <input
          type="text"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="A brief description…"
          className="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm text-white placeholder:text-zinc-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
      </div>
      <label className="flex items-center gap-2 text-sm text-zinc-200">
        <input
          type="checkbox"
          checked={isPublic}
          onChange={(e) => setIsPublic(e.target.checked)}
          className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-indigo-500"
        />
        Make publicly shareable
      </label>
      {error && (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" size="medium" disabled={saving}>
          {saving ? "Creating…" : "Create"}
        </Button>
        <Button
          type="button"
          size="medium"
          variant="tertiary"
          onClick={() => setOpen(false)}
        >
          Cancel
        </Button>
      </div>
    </form>
  );
}

// ─── Share link panel ─────────────────────────────────────────────────────────

function SharePanel({ watchlist }: { watchlist: Watchlist }) {
  const [copied, setCopied] = useState(false);

  if (!watchlist.public || !watchlist.public_slug) return null;

  const shareUrl = publicWatchlistShareUrl(watchlist.public_slug);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(shareUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard API not available
    }
  }

  return (
    <div className="flex items-center gap-2 mt-2 p-2 rounded bg-zinc-800/50 border border-zinc-700">
      <span className="text-xs text-zinc-400 truncate flex-1" title={shareUrl}>
        {shareUrl}
      </span>
      <button
        type="button"
        onClick={copyLink}
        className="shrink-0 text-xs text-indigo-400 hover:text-indigo-300 transition-colors"
        aria-label="Copy shareable link"
      >
        {copied ? "Copied!" : "Copy link"}
      </button>
    </div>
  );
}

// ─── Watchlist card ───────────────────────────────────────────────────────────

function WatchlistCard({
  watchlist,
  address,
  bearerToken,
  onUpdated,
  onDeleted,
}: {
  watchlist: Watchlist;
  address: string;
  bearerToken: string;
  onUpdated: (wl: Watchlist) => void;
  onDeleted: (id: number) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [editName, setEditName] = useState(watchlist.name);
  const [editDesc, setEditDesc] = useState(watchlist.description ?? "");
  const [editPublic, setEditPublic] = useState(watchlist.public);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [addPoolId, setAddPoolId] = useState("");
  const [addError, setAddError] = useState<string | null>(null);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      const updated = await updateWatchlist(address, watchlist.id, bearerToken, {
        name: editName.trim() || watchlist.name,
        description: editDesc.trim() || null,
        public: editPublic,
      });
      onUpdated(updated);
      setEditing(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save.");
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (!window.confirm(`Delete watchlist "${watchlist.name}"?`)) return;
    try {
      await deleteWatchlist(address, watchlist.id, bearerToken);
      onDeleted(watchlist.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete.");
    }
  }

  async function handleAddMarket(e: React.FormEvent) {
    e.preventDefault();
    const id = parseInt(addPoolId.trim(), 10);
    if (!addPoolId.trim() || isNaN(id) || id <= 0) {
      setAddError("Enter a valid pool ID.");
      return;
    }
    setAddError(null);
    try {
      await addMarketToWatchlist(address, watchlist.id, id, bearerToken);
      // Refresh the watchlist to show the new market.
      const updated = await updateWatchlist(address, watchlist.id, bearerToken, {});
      onUpdated(updated);
      setAddPoolId("");
    } catch (err) {
      setAddError(err instanceof Error ? err.message : "Failed to add market.");
    }
  }

  async function handleRemoveMarket(poolId: number) {
    try {
      await removeMarketFromWatchlist(address, watchlist.id, poolId, bearerToken);
      const updated = await updateWatchlist(address, watchlist.id, bearerToken, {});
      onUpdated(updated);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to remove market.");
    }
  }

  return (
    <div className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-4 space-y-3">
      {/* Header */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          {editing ? (
            <Input
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              autoFocus
              className="text-base font-semibold"
            />
          ) : (
            <h3 className="text-base font-semibold text-white truncate">{watchlist.name}</h3>
          )}
          {!editing && watchlist.description && (
            <p className="text-xs text-zinc-500 mt-0.5">{watchlist.description}</p>
          )}
        </div>
        <div className="flex gap-2 shrink-0">
          {editing ? (
            <>
              <Button size="small" onClick={handleSave} disabled={saving}>
                {saving ? "Saving…" : "Save"}
              </Button>
              <Button size="small" variant="tertiary" onClick={() => setEditing(false)}>
                Cancel
              </Button>
            </>
          ) : (
            <>
              <button
                type="button"
                onClick={() => setEditing(true)}
                className="text-xs text-zinc-500 hover:text-zinc-300 transition-colors"
              >
                Edit
              </button>
              <button
                type="button"
                onClick={handleDelete}
                className="text-xs text-zinc-500 hover:text-red-400 transition-colors"
                aria-label={`Delete watchlist ${watchlist.name}`}
              >
                Delete
              </button>
            </>
          )}
        </div>
      </div>

      {/* Edit panel */}
      {editing && (
        <div className="space-y-2">
          <input
            type="text"
            value={editDesc}
            onChange={(e) => setEditDesc(e.target.value)}
            placeholder="Description (optional)"
            className="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm text-white placeholder:text-zinc-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <label className="flex items-center gap-2 text-sm text-zinc-200">
            <input
              type="checkbox"
              checked={editPublic}
              onChange={(e) => setEditPublic(e.target.checked)}
              className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-indigo-500"
            />
            Publicly shareable
          </label>
          {!editPublic && watchlist.public && (
            <p className="text-xs text-amber-500" role="status">
              Disabling will revoke the current share link immediately.
            </p>
          )}
        </div>
      )}

      {/* Share panel */}
      <SharePanel watchlist={watchlist} />

      {/* Privacy badge */}
      <div>
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-[10px] font-medium",
            watchlist.public
              ? "bg-emerald-400/10 text-emerald-400"
              : "bg-zinc-800 text-zinc-500",
          )}
        >
          {watchlist.public ? "Public" : "Private"}
        </span>
        <span className="ml-2 text-[10px] text-zinc-600">
          {watchlist.market_ids.length} market{watchlist.market_ids.length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Markets list */}
      {watchlist.market_ids.length > 0 && (
        <ul className="space-y-1" aria-label={`Markets in ${watchlist.name}`}>
          {watchlist.market_ids.map((poolId) => (
            <li
              key={poolId}
              className="flex items-center justify-between gap-2 rounded bg-zinc-800/50 px-3 py-1.5"
            >
              <a
                href={`/user/pool-market/${poolId}`}
                className="text-sm text-zinc-300 hover:text-white transition-colors"
              >
                Pool #{poolId}
              </a>
              <button
                type="button"
                onClick={() => handleRemoveMarket(poolId)}
                className="text-[11px] text-zinc-500 hover:text-red-400 transition-colors"
                aria-label={`Remove pool ${poolId} from watchlist`}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      {/* Add market form */}
      <form onSubmit={handleAddMarket} className="flex gap-2 items-center">
        <input
          type="number"
          value={addPoolId}
          onChange={(e) => setAddPoolId(e.target.value)}
          placeholder="Pool ID"
          aria-label="Pool ID to add to watchlist"
          className="w-24 rounded-md border border-zinc-700 bg-zinc-900 px-2 py-1.5 text-sm text-white placeholder:text-zinc-600 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-zinc-600"
        />
        <button
          type="submit"
          className="text-xs text-indigo-400 hover:text-indigo-300 transition-colors"
        >
          + Add market
        </button>
        {addError && (
          <span className="text-xs text-destructive" role="alert">
            {addError}
          </span>
        )}
      </form>

      {/* Errors */}
      {error && (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

export function WatchlistManager({ address, bearerToken }: WatchlistManagerProps) {
  const {
    data: watchlists,
    isLoading,
    error,
    mutate,
  } = useSWR(
    watchlistsUrl(address),
    () => fetchWatchlists(address, bearerToken),
    { revalidateOnFocus: false },
  );

  function handleCreated(wl: Watchlist) {
    mutate(
      (prev) => (prev ? [...prev, wl] : [wl]),
      { revalidate: false },
    );
  }

  function handleUpdated(wl: Watchlist) {
    mutate(
      (prev) => prev?.map((w) => (w.id === wl.id ? wl : w)),
      { revalidate: false },
    );
  }

  function handleDeleted(id: number) {
    mutate(
      (prev) => prev?.filter((w) => w.id !== id),
      { revalidate: false },
    );
  }

  return (
    <div className="space-y-4">
      {/* Create button / form */}
      <CreateWatchlistForm
        address={address}
        bearerToken={bearerToken}
        onCreated={handleCreated}
      />

      {/* Loading */}
      {isLoading && (
        <div className="space-y-3" aria-busy="true" aria-label="Loading watchlists">
          {Array.from({ length: 2 }).map((_, i) => (
            <div key={i} className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-4 h-24 animate-pulse" />
          ))}
        </div>
      )}

      {/* Error */}
      {error && !isLoading && (
        <p className="text-sm text-zinc-500" role="alert">
          Could not load watchlists.
        </p>
      )}

      {/* Empty state */}
      {!isLoading && !error && watchlists?.length === 0 && (
        <p className="text-sm text-zinc-500" role="status">
          No watchlists yet. Create one above to start curating markets.
        </p>
      )}

      {/* Watchlist cards */}
      {watchlists?.map((wl) => (
        <WatchlistCard
          key={wl.id}
          watchlist={wl}
          address={address}
          bearerToken={bearerToken}
          onUpdated={handleUpdated}
          onDeleted={handleDeleted}
        />
      ))}
    </div>
  );
}

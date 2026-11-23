"use client";

/**
 * Opt-in creator profile form with provenance links (Issue #278).
 *
 * Features:
 * - Display name, avatar URL, bio, and up to 10 provenance links.
 * - Public/private toggle — private by default.
 * - Provenance links show a "pending verification" badge; a checkmark once verified.
 * - Saves via PUT /api/v1/users/:address/creator-profile.
 */

import { useState } from "react";
import { Button, Input } from "@/components/ui";
import {
  type CreatorProfile,
  type ProvenancePlatform,
  type CreatorProfileProof,
  CREATOR_PROFILE_MAX_BIO,
  CREATOR_PROFILE_MAX_DISPLAY_NAME,
  CREATOR_PROFILE_MAX_HANDLE_OR_URL,
  CREATOR_PROFILE_MAX_PROVENANCE_LINKS,
  PROVENANCE_PLATFORMS,
} from "@/lib/api/creatorProfile";

// ─── Local form types ─────────────────────────────────────────────────────────

interface LinkDraft {
  platform: ProvenancePlatform;
  handle_or_url: string;
  /** Reflects backend verification state; always false for newly added links. */
  verified: boolean;
}

interface FormValues {
  display_name: string;
  avatar_url: string;
  bio: string;
  provenance_links: LinkDraft[];
  public: boolean;
}

interface FormErrors {
  display_name?: string;
  avatar_url?: string;
  bio?: string;
  provenance_links?: string;
  links?: Record<number, string>;
}

// ─── Validation ───────────────────────────────────────────────────────────────

function validate(values: FormValues): FormErrors {
  const errors: FormErrors = {};

  if (values.display_name.length > CREATOR_PROFILE_MAX_DISPLAY_NAME) {
    errors.display_name = `Display name must be ${CREATOR_PROFILE_MAX_DISPLAY_NAME} characters or fewer.`;
  }

  if (values.avatar_url) {
    if (!values.avatar_url.startsWith("https://")) {
      errors.avatar_url = "Avatar URL must start with https://";
    } else if (values.avatar_url.length > 512) {
      errors.avatar_url = "Avatar URL must be 512 characters or fewer.";
    }
  }

  if (values.bio.length > CREATOR_PROFILE_MAX_BIO) {
    errors.bio = `Bio must be ${CREATOR_PROFILE_MAX_BIO} characters or fewer.`;
  }

  if (values.provenance_links.length > CREATOR_PROFILE_MAX_PROVENANCE_LINKS) {
    errors.provenance_links = `You may add at most ${CREATOR_PROFILE_MAX_PROVENANCE_LINKS} provenance links.`;
  }

  const linkErrors: Record<number, string> = {};
  values.provenance_links.forEach((link, i) => {
    if (!link.handle_or_url.trim()) {
      linkErrors[i] = "Handle or URL must not be empty.";
    } else if (link.handle_or_url.length > CREATOR_PROFILE_MAX_HANDLE_OR_URL) {
      linkErrors[i] = `Handle or URL must be ${CREATOR_PROFILE_MAX_HANDLE_OR_URL} characters or fewer.`;
    }
  });
  if (Object.keys(linkErrors).length > 0) {
    errors.links = linkErrors;
  }

  return errors;
}

// ─── Save error mapping ───────────────────────────────────────────────────────

/**
 * Maps backend error codes for ownership-proof failures to user-facing copy.
 * Falls back to the error message for unknown failures.
 */
function describeSaveError(err: unknown): string {
  const code =
    err && typeof err === "object" && "code" in err
      ? String((err as { code?: unknown }).code)
      : undefined;

  switch (code) {
    case "invalid_signature":
      return "Wallet signature could not be verified. Please sign again with the wallet that owns this profile.";
    case "replayed_proof":
    case "proof_replayed":
      return "This signature has already been used. Please sign a fresh challenge and try again.";
    case "revision_conflict":
    case "stale_revision":
      return "This profile was updated elsewhere. Reload the latest revision before saving.";
    case "wallet_mismatch":
      return "The signing wallet does not match the profile owner. Switch wallets and try again.";
    default:
      return err instanceof Error ? err.message : "Failed to save creator profile.";
  }
}

// ─── Component ────────────────────────────────────────────────────────────────

interface CreatorProfileFormProps {
  /** Currently saved profile (undefined when the creator hasn't opted in yet). */
  initialProfile?: CreatorProfile | null;
  /** Bearer token for the authenticated wallet — required to save. */
  bearerToken: string;
  /** Stellar address of the authenticated wallet. */
  address: string;
  /**
   * Requests a fresh wallet signature over the server-issued proof challenge
   * (nonce + timestamp). Must return the signed payload for the current wallet.
   */
  signProof: (challenge: CreatorProfileProofChallenge) => Promise<CreatorProfileProof>;
  /** Called after a successful save with the updated profile. */
  onSave?: (profile: CreatorProfile) => void;
}

/** Challenge material the backend issues for a fresh ownership proof. */
export interface CreatorProfileProofChallenge {
  nonce: string;
  timestamp: number;
  address: string;
}

export function CreatorProfileForm({
  initialProfile,
  bearerToken,
  address,
  signProof,
  onSave,
}: CreatorProfileFormProps) {
  const [values, setValues] = useState<FormValues>(() => ({
    display_name: initialProfile?.display_name ?? "",
    avatar_url: initialProfile?.avatar_url ?? "",
    bio: initialProfile?.bio ?? "",
    provenance_links:
      initialProfile?.provenance_links?.map((l) => ({
        platform: l.platform,
        handle_or_url: l.handle_or_url,
        verified: l.verified,
      })) ?? [],
    public: initialProfile?.public ?? false,
  }));

  const [errors, setErrors] = useState<FormErrors>({});
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [profile, setProfile] = useState<CreatorProfile | null>(initialProfile ?? null);

  // ─── Field helpers ──────────────────────────────────────────────────────────

  function handleChange<K extends keyof FormValues>(key: K, value: FormValues[K]) {
    setValues((prev) => ({ ...prev, [key]: value }));
    setErrors((prev) => ({ ...prev, [key]: undefined }));
    setSaved(false);
    setSaveError(null);
  }

  // ─── Provenance links ───────────────────────────────────────────────────────

  function addLink() {
    if (values.provenance_links.length >= CREATOR_PROFILE_MAX_PROVENANCE_LINKS) return;
    setValues((prev) => ({
      ...prev,
      provenance_links: [
        ...prev.provenance_links,
        { platform: "github", handle_or_url: "", verified: false },
      ],
    }));
  }

  function removeLink(index: number) {
    setValues((prev) => ({
      ...prev,
      provenance_links: prev.provenance_links.filter((_, i) => i !== index),
    }));
  }

  function updateLink(index: number, field: keyof Omit<LinkDraft, "verified">, value: string) {
    setValues((prev) => {
      const links = [...prev.provenance_links];
      links[index] = { ...links[index], [field]: value };
      return { ...prev, provenance_links: links };
    });
    setErrors((prev) => {
      const linkErrs = { ...(prev.links ?? {}) };
      delete linkErrs[index];
      return { ...prev, links: linkErrs };
    });
  }

  // ─── Submit ─────────────────────────────────────────────────────────────────

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const errs = validate(values);
    if (Object.keys(errs).length > 0) {
      setErrors(errs);
      return;
    }

    setSaving(true);
    setSaveError(null);
    try {
      const { upsertCreatorProfile, requestCreatorProfileChallenge } = await import(
        "@/lib/api/creatorProfile"
      );

      // 1. Obtain a fresh, single-use challenge from the backend.
      const challenge = await requestCreatorProfileChallenge(address, bearerToken);

      // 2. Have the wallet sign the fresh challenge (nonce + timestamp).
      const proof = await signProof({
        nonce: challenge.nonce,
        timestamp: challenge.timestamp,
        address,
      });

      // 3. Submit the profile together with the proof and expected revision.
      const updated = await upsertCreatorProfile(address, bearerToken, {
        display_name: values.display_name.trim() || null,
        avatar_url: values.avatar_url.trim() || null,
        bio: values.bio.trim() || null,
        provenance_links: values.provenance_links.map((l) => ({
          platform: l.platform,
          handle_or_url: l.handle_or_url.trim(),
        })),
        public: values.public,
        expected_revision: profile?.revision ?? null,
        proof,
      });
      setProfile(updated);
      setSaved(true);
      onSave?.(updated);
    } catch (err) {
      setSaveError(describeSaveError(err));
    } finally {
      setSaving(false);
    }
  }

  // ─── Render ─────────────────────────────────────────────────────────────────

  return (
    <form onSubmit={handleSubmit} noValidate className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-white">Creator Profile</h2>
        <p className="text-xs text-zinc-500">
          Opt in to show your identity alongside markets you create. Private by
          default — toggle public visibility when you're ready.
        </p>
        {profile && (
          <p className="text-xs text-zinc-500" data-testid="creator-profile-state">
            Revision {profile.revision} ·{" "}
            {profile.public ? "Public" : "Private"}
          </p>
        )}
      </div>

      {/* Display name */}
      <Input
        label="Display name"
        placeholder="Satoshi"
        value={values.display_name}
        onChange={(e) => handleChange("display_name", e.target.value)}
        error={errors.display_name}
        helperText={`${values.display_name.length}/${CREATOR_PROFILE_MAX_DISPLAY_NAME}`}
        autoComplete="name"
      />

      {/* Avatar URL */}
      <Input
        label="Avatar URL"
        type="url"
        placeholder="https://example.com/avatar.png"
        value={values.avatar_url}
        onChange={(e) => handleChange("avatar_url", e.target.value)}
        error={errors.avatar_url}
        helperText="Must start with https://"
        autoComplete="url"
      />

      {/* Bio */}
      <div className="space-y-2">
        <label className="text-sm font-medium text-zinc-200">Bio</label>
        <textarea
          rows={3}
          placeholder="Tell the community about yourself…"
          value={values.bio}
          onChange={(e) => handleChange("bio", e.target.value)}
          className="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm text-white placeholder:text-zinc-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 resize-none"
        />
        {errors.bio && (
          <p className="text-sm text-destructive" role="alert">
            {errors.bio}
          </p>
        )}
        <p className="text-xs text-zinc-500 text-right">
          {values.bio.length}/{CREATOR_PROFILE_MAX_BIO}
        </p>
      </div>

      {/* Provenance links */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <label className="text-sm font-medium text-zinc-200">
            Provenance links{" "}
            <span className="text-zinc-500 font-normal">
              ({values.provenance_links.length}/{CREATOR_PROFILE_MAX_PROVENANCE_LINKS})
            </span>
          </label>
          {values.provenance_links.length < CREATOR_PROFILE_MAX_PROVENANCE_LINKS && (
            <button
              type="button"
              onClick={addLink}
              className="text-xs text-indigo-400 hover:text-indigo-300 transition-colors"
            >
              + Add link
            </button>
          )}
        </div>

        {errors.provenance_links && (
          <p className="text-sm text-destructive" role="alert">
            {errors.provenance_links}
          </p>
        )}

        {values.provenance_links.length === 0 && (
          <p className="text-xs text-zinc-500">
            No links added. Click "Add link" to connect a social profile.
          </p>
        )}

        {values.provenance_links.map((link, i) => (
          <div key={i} className="flex gap-2 items-start">
            {/* Platform selector */}
            <select
              value={link.platform}
              onChange={(e) =>
                updateLink(i, "platform", e.target.value as ProvenancePlatform)
              }
              aria-label={`Platform for link ${i + 1}`}
              className="rounded-md border border-input bg-zinc-900 px-2 py-2 text-sm text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {PROVENANCE_PLATFORMS.map((p) => (
                <option key={p} value={p}>
                  {p.charAt(0).toUpperCase() + p.slice(1)}
                </option>
              ))}
            </select>

            {/* Handle / URL input */}
            <div className="flex-1 space-y-1">
              <input
                type="text"
                value={link.handle_or_url}
                onChange={(e) => updateLink(i, "handle_or_url", e.target.value)}
                placeholder={
                  link.platform === "url"
                    ? "https://your-site.com"
                    : "@handle"
                }
                aria-label={`Handle or URL for link ${i + 1}`}
                className="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm text-white placeholder:text-zinc-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              />
              {errors.links?.[i] && (
                <p className="text-xs text-destructive" role="alert">
                  {errors.links[i]}
                </p>
              )}
            </div>

            {/* Verification badge */}
            {link.verified ? (
              <span
                title="Verified"
                aria-label="Link verified"
                className="mt-2 text-green-400 text-sm select-none"
              >
                ✓
              </span>
            ) : (
              <span
                title="Pending verification"
                aria-label="Pending verification"
                className="mt-2 text-zinc-500 text-xs select-none"
              >
                ⏳
              </span>
            )}

            {/* Remove */}
            <button
              type="button"
              onClick={() => removeLink(i)}
              aria-label={`Remove link ${i + 1}`}
              className="mt-2 text-zinc-500 hover:text-red-400 transition-colors text-sm"
            >
              ✕
            </button>
          </div>
        ))}
      </div>

      {/* Public visibility toggle */}
      <div className="flex items-center gap-3">
        <input
          id="creator-profile-public"
          type="checkbox"
          checked={values.public}
          onChange={(e) => handleChange("public", e.target.checked)}
          className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-indigo-500 focus:ring-indigo-500"
        />
        <label htmlFor="creator-profile-public" className="text-sm text-zinc-200">
          Make profile publicly visible
        </label>
      </div>
      <p className="text-xs text-zinc-500 -mt-4">
        When enabled, your display name, bio, and verified provenance links appear
        beside markets you create.
      </p>

      {/* Save error */}
      {saveError && (
        <p className="text-sm text-destructive" role="alert">
          {saveError}
        </p>
      )}

      {/* Actions */}
      <div className="flex items-center gap-3">
        <Button type="submit" size="medium" disabled={saving}>
          {saving ? "Saving…" : "Save creator profile"}
        </Button>
        {saved && !saveError && (
          <span className="text-sm text-green-400" role="status">
            Saved!
          </span>
        )}
      </div>
    </form>
  );
}

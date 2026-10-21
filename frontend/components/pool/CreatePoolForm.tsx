"use client";

import React, { useState, useId, useRef, useEffect } from "react";
import { useRouter } from "next/navigation";
import { CheckCircle2, Info, Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button, Input, StakeInput, Checkbox, SupportedTokensPicker } from "@/components/ui";
import type { Token } from "@/components/ui";
import {
  validateCreatePool,
  minCloseTimeValue,
  POOL_CATEGORIES,
  MIN_OUTCOMES,
  MAX_OUTCOMES,
  type CreatePoolFormValues,
  type CreatePoolFormErrors,
} from "@/lib/validations/poolCreation";

// ─── Initial state ────────────────────────────────────────────────────────────

const INITIAL_VALUES: CreatePoolFormValues = {
  name: "",
  description: "",
  category: "",
  outcomes: ["", ""],
  minStake: "",
  maxStake: "",
  closeTime: "",
  token: "XLM",
  termsAccepted: false,
};

// ─── Section wrapper ──────────────────────────────────────────────────────────

function FormSection({
  title,
  description,
  children,
  sectionId,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
  sectionId?: string;
}) {
  return (
    <section id={sectionId} aria-label={title} className="space-y-4">
      <div className="space-y-0.5">
        <h2 className="text-sm font-semibold text-white tracking-wide uppercase">
          {title}
        </h2>
        {description && (
          <p className="text-xs text-zinc-500">{description}</p>
        )}
      </div>
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-3 sm:p-5 space-y-4 sm:space-y-5 min-w-0">
        {children}
      </div>
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-5 space-y-5">{children}</div>
    </section>
  );
}

// ─── Character counter ────────────────────────────────────────────────────────

function CharCount({ current, max }: { current: number; max: number }) {
  const overLimit = current > max;
  return (
    <span
      className={cn(
        "text-xs tabular-nums",
        overLimit ? "text-destructive font-medium" : "text-zinc-600",
      )}
      aria-live="polite"
    >
      {current}/{max}
    </span>
  );
}

// ─── Select field ─────────────────────────────────────────────────────────────

interface SelectFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: readonly string[];
  placeholder?: string;
  error?: string;
  disabled?: boolean;
}

function SelectField({
  id,
  label,
  value,
  onChange,
  options,
  placeholder = "Select an option",
  error,
  disabled,
}: SelectFieldProps) {
  return (
    <div className="w-full space-y-2">
      <label
        htmlFor={id}
        className={cn(
          "text-sm font-medium leading-none",
          error ? "text-destructive" : "text-zinc-200",
        )}
      >
        {label}
      </label>
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        aria-invalid={error ? "true" : "false"}
        aria-describedby={error ? `${id}-error` : undefined}
        className={cn(
          "flex h-10 w-full rounded-md border bg-transparent px-3 py-2 text-sm",
          "ring-offset-background",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "text-white [&>option]:bg-zinc-900 [&>option]:text-white",
          error ? "border-destructive focus-visible:ring-destructive" : "border-input",
        )}
      >
        <option value="" disabled className="text-zinc-500">
          {placeholder}
        </option>
        {options.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
      {error && (
        <p id={`${id}-error`} className="text-sm font-medium text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

// ─── Datetime field ───────────────────────────────────────────────────────────

interface DateTimeFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  min?: string;
  error?: string;
  helperText?: string;
  disabled?: boolean;
}

function DateTimeField({
  id,
  label,
  value,
  onChange,
  min,
  error,
  helperText,
  disabled,
}: DateTimeFieldProps) {
  return (
    <div className="w-full space-y-2">
      <label
        htmlFor={id}
        className={cn(
          "text-sm font-medium leading-none",
          error ? "text-destructive" : "text-zinc-200",
        )}
      >
        {label}
      </label>
      <input
        id={id}
        type="datetime-local"
        value={value}
        min={min}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        aria-invalid={error ? "true" : "false"}
        aria-describedby={error ? `${id}-error` : helperText ? `${id}-helper` : undefined}
        className={cn(
          "flex h-10 w-full rounded-md border bg-transparent px-3 py-2 text-sm text-white",
          "ring-offset-background",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "[color-scheme:dark]",
          error ? "border-destructive focus-visible:ring-destructive" : "border-input",
        )}
      />
      {error && (
        <p id={`${id}-error`} className="text-sm font-medium text-destructive" role="alert">
          {error}
        </p>
      )}
      {!error && helperText && (
        <p id={`${id}-helper`} className="text-xs text-zinc-500">
          {helperText}
        </p>
      )}
    </div>
  );
}

// ─── Textarea field ───────────────────────────────────────────────────────────

interface TextareaFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  maxLength?: number;
  rows?: number;
  error?: string;
  helperText?: string;
  disabled?: boolean;
}

function TextareaField({
  id,
  label,
  value,
  onChange,
  placeholder,
  maxLength,
  rows = 3,
  error,
  helperText,
  disabled,
}: TextareaFieldProps) {
  return (
    <div className="w-full space-y-2">
      <label
        htmlFor={id}
        className={cn(
          "text-sm font-medium leading-none",
          error ? "text-destructive" : "text-zinc-200",
        )}
      >
        {label}
      </label>
      <textarea
        id={id}
        rows={rows}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        aria-invalid={error ? "true" : "false"}
        aria-describedby={error ? `${id}-error` : helperText ? `${id}-helper` : undefined}
        className={cn(
          "w-full rounded-md border bg-transparent px-3 py-2 text-sm text-white",
          "placeholder:text-zinc-500 ring-offset-background resize-none",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          error ? "border-destructive focus-visible:ring-destructive" : "border-input",
        )}
      />
      <div className="flex items-center justify-between gap-2">
        {error ? (
          <p id={`${id}-error`} className="text-sm font-medium text-destructive" role="alert">
            {error}
          </p>
        ) : helperText ? (
          <p id={`${id}-helper`} className="text-xs text-zinc-500">
            {helperText}
          </p>
        ) : (
          <span />
        )}
        {maxLength != null && <CharCount current={value.length} max={maxLength} />}
      </div>
    </div>
  );
}

// ─── Success state ────────────────────────────────────────────────────────────

function SuccessState({ onCreateAnother }: { onCreateAnother: () => void }) {
  const router = useRouter();
  return (
    <div className="flex flex-col items-center justify-center gap-6 rounded-xl border border-zinc-800 bg-zinc-900 p-12 text-center">
      <div className="flex h-16 w-16 items-center justify-center rounded-full bg-[#37B7C3]/10">
        <CheckCircle2 className="h-8 w-8 text-[#37B7C3]" aria-hidden="true" />
      </div>
      <div className="space-y-2">
        <h2 className="text-xl font-semibold text-white">Pool Created Successfully</h2>
        <p className="max-w-sm text-sm text-zinc-400">
          Your prediction pool has been submitted. It will appear in the market once confirmed on
          the Stellar network.
        </p>
      </div>
      <div className="flex flex-col sm:flex-row gap-3">
        <Button variant="primary" size="medium" onClick={() => router.push("/user/pool-market")}>
          View Pool Market
        </Button>
        <Button variant="tertiary" size="medium" onClick={onCreateAnother}>
          Create Another Pool
        </Button>
      </div>
    </div>
  );
}

// ─── Main form ────────────────────────────────────────────────────────────────

/**
 * CreatePoolForm
 *
 * Full prediction pool creation form with client-side validation.
 * Follows the manual validation pattern used throughout the project.
 */
export function CreatePoolForm() {
  const descriptionId = useId();
  const categoryId = useId();
  const closeTimeId = useId();
  const outcomesSectionId = useId();
  const outcomesErrorId = useId();

  const [values, setValues] = useState<CreatePoolFormValues>(INITIAL_VALUES);
  const [errors, setErrors] = useState<CreatePoolFormErrors>({});
  const [status, setStatus] = useState<"idle" | "submitting" | "success">("idle");
  const [submitError, setSubmitError] = useState<string | null>(null);

  // ── Focus management for dynamic outcome rows ──────────────────────────────
  /**
   * When an outcome is added we move focus to the new input. When removed we
   * focus the previous outcome input (or the add button if none remain).
   * `focusAction` records what happened and triggers the useEffect.
   */
  const outcomeRefs = useRef<(HTMLInputElement | null)[]>([]);
  const addButtonRef = useRef<HTMLButtonElement | null>(null);
  const [focusAction, setFocusAction] = useState<
    { type: "added"; index: number } | { type: "removed"; index: number } | null
  >(null);

  useEffect(() => {
    if (!focusAction) return;
    if (focusAction.type === "added") {
      const el = outcomeRefs.current[focusAction.index];
      el?.focus();
    } else {
      // Focus previous outcome, or the add button if we removed the first one
      const prevIndex = Math.max(0, focusAction.index - 1);
      const el = outcomeRefs.current[prevIndex];
      if (el) {
        el.focus();
      } else {
        addButtonRef.current?.focus();
      }
    }
    setFocusAction(null);
  }, [focusAction, values.outcomes.length]);

  const isSubmitting = status === "submitting";

  // ── Field updaters ──────────────────────────────────────────────────────────

  function setField<K extends keyof CreatePoolFormValues>(
    field: K,
    value: CreatePoolFormValues[K],
  ) {
    setValues((prev) => ({ ...prev, [field]: value }));
    // Clear error on change
    if (errors[field]) {
      setErrors((prev) => ({ ...prev, [field]: undefined }));
    }
    setSubmitError(null);
  }

  function setOutcome(index: number, value: string) {
    const outcomes = [...values.outcomes];
    outcomes[index] = value;
    setValues((prev) => {
      return { ...prev, outcomes };
    });
    setErrors((prev) => {
      if (!prev.outcomeErrors && !prev.outcomes) return prev;
      const updatedErrors = validateCreatePool({ ...values, outcomes });
      return {
        ...prev,
        outcomeErrors: updatedErrors.outcomeErrors,
        outcomes: updatedErrors.outcomes,
      };
    });
    setSubmitError(null);
  }

  function addOutcome() {
    if (values.outcomes.length >= MAX_OUTCOMES) return;
    const outcomes = [...values.outcomes, ""];
    setValues((prev) => ({ ...prev, outcomes }));
    setErrors((prev) => {
      if (!prev.outcomeErrors && !prev.outcomes) return prev;
      const updatedErrors = validateCreatePool({ ...values, outcomes });
      return {
        ...prev,
        outcomeErrors: updatedErrors.outcomeErrors,
        outcomes: updatedErrors.outcomes,
      };
    });
    setFocusAction({ type: "added", index: values.outcomes.length });
  }

  function removeOutcome(index: number) {
    if (values.outcomes.length <= MIN_OUTCOMES) return;
    const outcomes = values.outcomes.filter((_, i) => i !== index);
    setValues((prev) => ({ ...prev, outcomes }));
    setErrors((prev) => {
      if (!prev.outcomeErrors && !prev.outcomes) return prev;
      const updatedErrors = validateCreatePool({ ...values, outcomes });
      return {
        ...prev,
        outcomeErrors: updatedErrors.outcomeErrors,
        outcomes: updatedErrors.outcomes,
      };
    });
    setFocusAction({ type: "removed", index });
  }

  function handleTokenChange(token: Token) {
    setValues((prev) => ({ ...prev, token: token.id }));
    // Re-validate stake fields when token changes
    setErrors((prev) => ({
      ...prev,
      minStake: undefined,
      maxStake: undefined,
    }));
  }

  // ── Submit ──────────────────────────────────────────────────────────────────

  async function handleSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();

    const validationErrors = validateCreatePool(values);
    if (Object.keys(validationErrors).length > 0) {
      setErrors(validationErrors);

      // Determine the first field that has an error and focus it.
      // The form fields are checked in DOM order so focus lands on the
      // top-most error the user needs to fix.
      // Uses getElementById (not querySelector) because React's useId()
      // generates IDs containing colons (":r0:") which are invalid in
      // CSS selectors.
      const fieldIdMap: { key: keyof CreatePoolFormErrors; id: string }[] = [
        { key: "name", id: "name" },
        { key: "description", id: descriptionId },
        { key: "category", id: categoryId },
        { key: "tags", id: "tags" },
        {
          key: "outcomeErrors",
          id: `outcome-${validationErrors.outcomeErrors?.findIndex((err) => err) ?? 0}`,
        },
        { key: "outcomes", id: outcomesErrorId },
        { key: "minStake", id: "minStake" },
        { key: "maxStake", id: "maxStake" },
        { key: "closeTime", id: closeTimeId },
        { key: "termsAccepted", id: "termsAccepted" },
      ];

      const firstError = fieldIdMap.find(({ key }) => validationErrors[key]);
      if (firstError) {
        const el = document.getElementById(firstError.id);
        if (el) {
          el.scrollIntoView({ behavior: "smooth", block: "center" });
          // Focus the element so keyboard/screen-reader users land on it.
          // Inputs/selects/textareas are focusable directly; for the
          // section-level outcomes error we focus the add button.
          if (firstError.key === "outcomes") {
            addButtonRef.current?.focus();
          } else {
            el.focus?.();
          }
        }
      }
      return;
    }

    setStatus("submitting");
    setSubmitError(null);

    try {
      // TODO: replace with actual contract / API call
      await new Promise<void>((resolve) => setTimeout(resolve, 1500));
      setStatus("success");
    } catch {
      setStatus("idle");
      setSubmitError("Something went wrong submitting your pool. Please try again.");
    }
  }

  function handleReset() {
    setValues(INITIAL_VALUES);
    setErrors({});
    setStatus("idle");
    setSubmitError(null);
  }

  // ── Success screen ──────────────────────────────────────────────────────────

  if (status === "success") {
    return <SuccessState onCreateAnother={handleReset} />;
  }

  // ── Form ────────────────────────────────────────────────────────────────────

  return (
    <form
      onSubmit={handleSubmit}
      noValidate
      aria-label="Create prediction pool"
      className="space-y-8 w-full max-w-full min-w-0 overflow-x-hidden"
    >
      {/* ── 1. Pool Details ── */}
      <FormSection
        title="Pool Details"
        description="Give your prediction pool a clear name and description."
      >
        <Input
          id="name"
          label="Pool Name"
          placeholder="e.g. Will BTC reach $100K by end of year?"
          value={values.name}
          onChange={(e) => setField("name", e.target.value)}
          error={errors.name}
          helperText="5–80 characters."
          disabled={isSubmitting}
          autoComplete="off"
        />

        <div className="space-y-2">
          <TextareaField
            id={descriptionId}
            label="Description"
            value={values.description}
            onChange={(val) => setField("description", val)}
            placeholder="Describe what participants are predicting and what determines the outcome…"
            maxLength={300}
            rows={4}
            error={errors.description}
            helperText="10–300 characters."
            disabled={isSubmitting}
          />
        </div>

        <SelectField
          id={categoryId}
          label="Category"
          value={values.category}
          onChange={(val) => setField("category", val as CreatePoolFormValues["category"])}
          options={POOL_CATEGORIES}
          placeholder="Select a category"
          error={errors.category}
          disabled={isSubmitting}
        />

        <Input
          id="tags"
          label="Tags (optional)"
          placeholder="e.g. btc, price-prediction, halving"
          value={values.tags}
          onChange={(e) => setField("tags", e.target.value)}
          helperText="Comma-separated. Helps participants filter and find your pool."
          disabled={isSubmitting}
          autoComplete="off"
        />
      </FormSection>

      {/* ── 2. Prediction Options ── */}
      <FormSection
        title="Prediction Options"
        description={`Define the outcomes participants can stake on (${MIN_OUTCOMES}–${MAX_OUTCOMES}).`}
        sectionId={outcomesSectionId}
      >
        <div
          className="space-y-3"
          role="group"
          aria-label="Outcomes"
          aria-describedby={errors.outcomes ? outcomesErrorId : undefined}
        >
          {values.outcomes.map((outcome, index) => (
            <div key={index} className="flex items-start gap-2 min-w-0">
              <div className="min-w-0 flex-1">
                <Input
                  id={`outcome-${index}`}
                  label={`Outcome ${index + 1}`}
                  placeholder={index === 0 ? "e.g. Yes" : index === 1 ? "e.g. No" : "e.g. Draw"}
                  value={outcome}
                  onChange={(e) => setOutcome(index, e.target.value)}
                  error={errors.outcomeErrors?.[index]}
                  disabled={isSubmitting}
                  autoComplete="off"
                  ref={(el) => {
                    outcomeRefs.current[index] = el;
                  }}
                />
              </div>
              {values.outcomes.length > MIN_OUTCOMES && (
                <button
                  type="button"
                  onClick={() => removeOutcome(index)}
                  disabled={isSubmitting}
                  aria-label={`Remove outcome ${index + 1}`}
                  className="mt-7 flex h-11 w-11 shrink-0 items-center justify-center rounded-md border border-zinc-700 text-zinc-400 hover:border-red-800 hover:text-red-400 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                  >
                  <X className="h-4 w-4" aria-hidden="true" />
                </button>
              )}
            </div>
          ))}

          {errors.outcomes && (
            <p id={outcomesErrorId} className="text-sm font-medium text-destructive" role="alert">
              {errors.outcomes}
            </p>
          )}

          {values.outcomes.length < MAX_OUTCOMES && (
            <Button
              ref={addButtonRef}
              type="button"
              variant="tertiary"
              size="small"
              onClick={addOutcome}
              disabled={isSubmitting}
              aria-describedby={errors.outcomes ? outcomesErrorId : undefined}
              className="gap-1.5"
            >
              <Plus className="h-4 w-4" aria-hidden="true" />
              Add outcome
            </Button>
          )}
        </div>
      </FormSection>

      {/* ── 3. Stake Configuration ── */}
      <FormSection
        title="Stake Configuration"
        description="Set the token and stake limits for pool participants."
      >
        {/* Token picker row */}
       <div className="flex flex-col gap-3 rounded-lg border border-zinc-700/50 bg-zinc-800/40 px-3 py-3 sm:flex-row sm:items-center sm:justify-between sm:px-4 min-w-0">
          <div className="space-y-0.5 min-w-0">
            <p className="text-sm font-medium text-zinc-200">Staking Token</p>
            <p className="text-xs text-zinc-500">Participants will stake in this token.</p>
          </div>
          <div className="w-full sm:w-auto shrink-0">
            <SupportedTokensPicker
              value={values.token}
              onChange={handleTokenChange}
              disabled={isSubmitting}
              className="w-full sm:w-auto"
            />
          </div>
        </div>
          <SupportedTokensPicker
            value={values.token}
            onChange={handleTokenChange}
            disabled={isSubmitting}
          />
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <StakeInput
            id="minStake"
            label="Minimum Stake"
            value={values.minStake}
            onChange={(raw) => setField("minStake", raw)}
            token={values.token}
            error={errors.minStake}
            helperText={`Minimum amount per participant.`}
            disabled={isSubmitting}
          />
          <StakeInput
            id="maxStake"
            label="Maximum Stake"
            value={values.maxStake}
            onChange={(raw) => setField("maxStake", raw)}
            token={values.token}
            error={errors.maxStake}
            helperText="Maximum amount per participant."
            disabled={isSubmitting}
          />
        </div>
      </FormSection>

      {/* ── 4. Pool Schedule ── */}
      <FormSection title="Pool Schedule" description="Set when the pool closes to new predictions.">
        <DateTimeField
          id={closeTimeId}
          label="Close Time"
          value={values.closeTime}
          onChange={(val) => setField("closeTime", val)}
          min={minCloseTimeValue()}
          error={errors.closeTime}
          helperText="Must be at least 30 minutes from now."
          disabled={isSubmitting}
        />

        {/* Informational hint */}
        <div className="flex items-start gap-2 rounded-lg bg-[#37B7C3]/5 border border-[#37B7C3]/15 p-3">
          <Info className="mt-0.5 h-4 w-4 shrink-0 text-[#7DE3EC]" aria-hidden="true" />
          <p className="text-xs text-zinc-400 leading-relaxed">
            After the close time, no new stakes are accepted. The pool outcome is settled by a
            designated validator on the Stellar network.
          </p>
        </div>
      </FormSection>

      {/* ── 5. Terms ── */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-3 sm:p-5 min-w-0">
        <div className="flex items-start gap-3">
          <Checkbox
            id="termsAccepted"
            checked={values.termsAccepted}
            onCheckedChange={(checked) => setField("termsAccepted", checked === true)}
            disabled={isSubmitting}
            aria-describedby={errors.termsAccepted ? "termsAccepted-error" : undefined}
            className="mt-0.5"
          />
          <label
            htmlFor="termsAccepted"
            className="text-sm text-zinc-300 leading-relaxed cursor-pointer select-none"
          >
            I understand that this pool is governed by smart contracts on the Stellar network and
            that outcomes are final once settled.
          </label>
        </div>
        {errors.termsAccepted && (
          <p
            id="termsAccepted-error"
            className="mt-2 ml-7 text-sm font-medium text-destructive"
            role="alert"
          >
            {errors.termsAccepted}
          </p>
        )}
      </div>

      {/* ── Submit error ── */}
      {submitError && (
        <div className="rounded-lg border border-red-800 bg-red-900/20 p-4" role="alert">
          <p className="text-sm text-red-400 text-center">{submitError}</p>
        </div>
      )}

      {/* ── Review summary (before wallet / submit) ── */}
      <section
        aria-label="Review pool before creating"
        className="rounded-xl border border-zinc-700 bg-zinc-900/80 p-3 sm:p-5 space-y-3 min-w-0"
      >
        <h2 className="text-sm font-semibold text-white tracking-wide uppercase">
          Review before create
        </h2>
        <dl className="grid grid-cols-1 gap-2 text-sm">
          <div className="flex flex-col gap-0.5 min-w-0 sm:flex-row sm:justify-between sm:gap-4">
            <dt className="text-zinc-500 shrink-0">Name</dt>
            <dd className="text-zinc-200 break-words text-left sm:text-right min-w-0">
              {values.name.trim() || "—"}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5 min-w-0 sm:flex-row sm:justify-between sm:gap-4">
            <dt className="text-zinc-500 shrink-0">Category</dt>
            <dd className="text-zinc-200 break-words sm:text-right">
              {values.category || "—"}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5 min-w-0 sm:flex-row sm:justify-between sm:gap-4">
            <dt className="text-zinc-500 shrink-0">Outcomes</dt>
            <dd className="text-zinc-200 break-words sm:text-right">
              {values.outcomes.filter((o) => o.trim()).join(" · ") || "—"}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5 min-w-0 sm:flex-row sm:justify-between sm:gap-4">
            <dt className="text-zinc-500 shrink-0">Stake range</dt>
            <dd className="text-zinc-200 tabular-nums sm:text-right">
              {values.minStake || "—"} – {values.maxStake || "—"} {values.token}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5 min-w-0 sm:flex-row sm:justify-between sm:gap-4">
            <dt className="text-zinc-500 shrink-0">Closes</dt>
            <dd className="text-zinc-200 break-all sm:text-right">
              {values.closeTime || "—"}
            </dd>
          </div>
        </dl>
        <p className="text-xs text-zinc-500 leading-relaxed">
          Confirm these details before submitting. Your wallet will be asked to
          sign the create transaction.
        </p>
      </section>

      {/* ── Actions ── */}
      <div className="flex flex-col sm:flex-row items-center justify-end gap-3 pt-2">
        <Button
          type="button"
          variant="tertiary"
          size="medium"
          onClick={handleReset}
          disabled={isSubmitting}
        >
          Reset
        </Button>
        <Button
          type="submit"
          variant="primary"
          size="medium"
          loading={isSubmitting}
          className="sm:min-w-[160px] bg-[#37B7C3] text-black hover:bg-[#2aa0ac] font-semibold"
        >
          {isSubmitting ? "Creating Pool…" : "Create Pool"}
        </Button>
      </div>
    </form>
  );
}

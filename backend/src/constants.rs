//! Shared constants for the backend.
//!
//! This module centralises stable identifiers used across the service,
//! including the deterministic fixture identifiers used by the seed tooling
//! (see issue #142). Fixture identifiers are intentionally stable so that
//! repeated seeding produces the same dataset.

/// Environment variable that must be set to an explicit confirmation value
/// before any destructive seed operation is allowed to run.
pub const SEED_CONFIRM_ENV: &str = "PREDIFI_SEED_CONFIRM";

/// The exact value `SEED_CONFIRM_ENV` must contain to authorise a destructive
/// seed operation. Requiring a non-trivial token prevents accidental
/// confirmation via a bare `1`/`true`.
pub const SEED_CONFIRM_VALUE: &str = "yes-reset-fixtures";

/// Environment variable describing the deployment environment.
pub const APP_ENV_ENV: &str = "PREDIFI_ENV";

/// Environment name that is treated as production. Fixture loading is refused
/// by default when `APP_ENV_ENV` resolves to this value.
pub const PRODUCTION_ENV: &str = "production";

/// Stable namespace prefix for all fixture identifiers. Keeping a single
/// prefix makes fixtures easy to recognise and impossible to confuse with
/// real production data.
pub const FIXTURE_ID_PREFIX: &str = "fixture";

/// Stable fixture identifiers. These are fixed values (not random UUIDs) so
/// that repeated seeding produces byte-for-byte identical rows.
pub mod fixture_ids {
    /// Deterministic identifier for the primary fixture user.
    pub const USER_PRIMARY: &str = "fixture-user-primary";

    /// Deterministic identifier for the secondary fixture user.
    pub const USER_SECONDARY: &str = "fixture-user-secondary";

    /// Deterministic identifier for the primary fixture market.
    pub const MARKET_PRIMARY: &str = "fixture-market-primary";

    /// Deterministic identifier for the secondary fixture market.
    pub const MARKET_SECONDARY: &str = "fixture-market-secondary";
}

/// Returns `true` when the given environment name refers to production.
///
/// Fixture loading must be refused by default in production; callers should
/// use this helper rather than comparing strings inline.
pub fn is_production_env(env: &str) -> bool {
    env.eq_ignore_ascii_case(PRODUCTION_ENV)
}

/// Returns `true` when the supplied confirmation value authorises a
/// destructive seed operation.
pub fn is_seed_confirmed(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v == SEED_CONFIRM_VALUE)
}

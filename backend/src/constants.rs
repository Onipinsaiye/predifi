// backend/src/constants.rs

// ---------------------------------------------------------------------------
// Market quote simulation constants
// ---------------------------------------------------------------------------

/// Maximum age (in seconds) of a pool state snapshot that may be used to
/// produce a market quote. Snapshots older than this are considered stale and
/// the quote request must be rejected.
pub const QUOTE_MAX_STALENESS_SECS: u64 = 30;

/// Minimum allowed fee, expressed in basis points (1 bps = 0.01%).
pub const MIN_FEE_BPS: u32 = 0;

/// Maximum allowed fee, expressed in basis points (1 bps = 0.01%).
/// 1000 bps == 10%.
pub const MAX_FEE_BPS: u32 = 1_000;

/// Denominator used to convert basis points into a fractional fee.
/// fee_fraction = fee_bps / BPS_DENOMINATOR
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Scaling factor applied to outcome shares so that fractional shares can be
/// represented as integers. Shares are stored/returned scaled by this factor.
pub const SHARE_SCALE: u64 = 1_000_000;

/// Number of decimal places implied by `SHARE_SCALE`.
pub const SHARE_DECIMALS: u32 = 6;

/// Minimum input amount (in input units) accepted by the quote operation.
/// Zero or sub-unit inputs are rejected.
pub const MIN_QUOTE_INPUT: u64 = 1;

/// Maximum input amount (in input units) accepted by the quote operation.
/// Guards against overflow in the pricing math.
pub const MAX_QUOTE_INPUT: u64 = 1_000_000_000_000_000;

/// Maximum price impact, in basis points, that a quote may report before it
/// is considered unsupported / unsafe to sign. 5000 bps == 50%.
pub const MAX_PRICE_IMPACT_BPS: u32 = 5_000;

/// Supported pricing model identifier used by the quote operation.
/// Must match the contract reference implementation.
pub const PRICING_MODEL: &str = "constant-product";

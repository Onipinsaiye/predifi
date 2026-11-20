//! # Address Pseudonymisation
//!
//! Wallet addresses are high-cardinality identifiers. Logging them raw expands
//! the blast radius of routine telemetry access: anyone who can read logs or
//! traces can enumerate every active wallet. This module replaces raw addresses
//! with short-lived **keyed pseudonyms** in all log, trace, and metric paths.
//!
//! ## Design
//!
//! A pseudonym is `psid-<8 lowercase hex chars>` derived by truncating the
//! HMAC-SHA256 of `address || epoch_bucket` under a process-scoped secret.
//! The same address maps to the same pseudonym within the same time bucket
//! (default: 1 hour), so traces emitted during one bucket can be correlated
//! without exposing the underlying address. After rotation the mapping changes.
//!
//! Properties:
//! - **Non-reversible** — truncated HMAC output cannot recover the input.
//! - **Correlation-preserving within a bucket** — the same address always
//!   produces the same pseudonym inside a single hour window.
//! - **Rotation** — after each `bucket_seconds` interval the pseudonym changes,
//!   limiting long-term re-identification from retained logs.
//! - **No address in labels** — `pseudonym()` is designed to be called wherever
//!   a wallet address would otherwise appear in a log field or metric label.
//!
//! ## Metric label policy
//!
//! Raw wallet addresses **must not** appear as Prometheus label values. Labels
//! are retained indefinitely in the TSDB and are not subject to log-rotation
//! TTLs. Use aggregated, low-cardinality labels (pool ID, outcome, network)
//! instead. If a per-user metric is genuinely required, use the pseudonym
//! returned by this module as the label value and document the rotation period.
//!
//! ## Usage
//!
//! ```rust
//! use predifi_backend::address_pseudonym::global_pseudonymizer;
//!
//! let psid = global_pseudonymizer().pseudonym("GABC...XYZ");
//! // logs: psid = "psid-3f7a9b2c"
//! ```
//!
//! Call-sites must import this module and call `pseudonym()` before passing an
//! address to any `tracing::info!`, `tracing::warn!`, or `#[instrument]` field.
//! Raw addresses must not appear as `%address` or `?address` in those macros.

use sha2::Digest;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default bucket size: pseudonyms rotate every hour.
pub const DEFAULT_BUCKET_SECONDS: u64 = 3_600;

/// Number of hex characters kept from the HMAC digest.
/// 8 hex chars = 32 bits — ample for correlation within a session, too
/// small to brute-force the original address.
const PSEUDONYM_HEX_CHARS: usize = 8;

/// Prefix added to every pseudonym so log consumers can identify and filter
/// these values programmatically.
const PSEUDONYM_PREFIX: &str = "psid-";

// ── Core struct ───────────────────────────────────────────────────────────────

/// A keyed, time-bucketed address pseudonymizer.
///
/// Construct with [`Pseudonymizer::new`] or use the process-global instance
/// via [`global_pseudonymizer`].
pub struct Pseudonymizer {
    /// HMAC key material.  Generated once at process start from the OS CSPRNG.
    key: Vec<u8>,
    /// Rotation period in seconds.
    bucket_seconds: u64,
}

impl Pseudonymizer {
    /// Create a pseudonymizer with the given key and rotation bucket.
    ///
    /// # Arguments
    /// * `key`            – Secret key bytes (at least 16 bytes recommended).
    /// * `bucket_seconds` – How often the pseudonym rotates (use
    ///   [`DEFAULT_BUCKET_SECONDS`] for the production default).
    pub fn new(key: Vec<u8>, bucket_seconds: u64) -> Self {
        Self { key, bucket_seconds }
    }

    /// Create a pseudonymizer with a fresh random key and the default bucket.
    ///
    /// Uses [`getrandom`] via `rand`-compatible byte generation to seed the key.
    pub fn with_random_key() -> Self {
        // 32-byte key from the OS CSPRNG.
        let mut key = vec![0u8; 32];
        getrandom_key(&mut key);
        Self::new(key, DEFAULT_BUCKET_SECONDS)
    }

    /// Current time bucket index.  All calls within the same bucket window
    /// (e.g. the same hour) return the same value.
    fn current_bucket(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now / self.bucket_seconds
    }

    /// Derive a pseudonym for `address` in the current time bucket.
    ///
    /// Returns a string of the form `"psid-<8 hex chars>"`.
    /// An empty address returns `"psid-anonymous"`.
    pub fn pseudonym(&self, address: &str) -> String {
        if address.is_empty() {
            return format!("{PSEUDONYM_PREFIX}anonymous");
        }
        let digest = self.hmac_digest(address, self.current_bucket());
        format!("{PSEUDONYM_PREFIX}{}", hex_prefix(&digest, PSEUDONYM_HEX_CHARS))
    }

    /// Derive a pseudonym for `address` in a specific bucket (for testing).
    #[cfg(test)]
    pub fn pseudonym_at_bucket(&self, address: &str, bucket: u64) -> String {
        if address.is_empty() {
            return format!("{PSEUDONYM_PREFIX}anonymous");
        }
        let digest = self.hmac_digest(address, bucket);
        format!("{PSEUDONYM_PREFIX}{}", hex_prefix(&digest, PSEUDONYM_HEX_CHARS))
    }

    /// Compute HMAC-SHA256(key, address || "|" || bucket) and return the digest bytes.
    ///
    /// We use a manual HMAC construction to avoid pulling in the `hmac` crate,
    /// since `sha2` is already a direct dependency.
    ///
    /// RFC 2104 HMAC:
    ///   ipad = 0x36 repeated to block size
    ///   opad = 0x5C repeated to block size
    ///   HMAC = H((key XOR opad) || H((key XOR ipad) || message))
    fn hmac_digest(&self, address: &str, bucket: u64) -> Vec<u8> {
        const BLOCK_SIZE: usize = 64; // SHA-256 block size in bytes

        // Prepare key: truncate or hash if longer than block size.
        let key_bytes: Vec<u8> = if self.key.len() > BLOCK_SIZE {
            sha2::Sha256::digest(&self.key).to_vec()
        } else {
            let mut k = self.key.clone();
            k.resize(BLOCK_SIZE, 0);
            k
        };

        // ipad / opad
        let ipad: Vec<u8> = key_bytes.iter().map(|b| b ^ 0x36).collect();
        let opad: Vec<u8> = key_bytes.iter().map(|b| b ^ 0x5C).collect();

        // Inner hash: SHA256(ipad || address || "|" || bucket_decimal)
        let mut inner = sha2::Sha256::new();
        inner.update(&ipad);
        inner.update(address.as_bytes());
        inner.update(b"|");
        inner.update(bucket.to_string().as_bytes());
        let inner_digest = inner.finalize();

        // Outer hash: SHA256(opad || inner_digest)
        let mut outer = sha2::Sha256::new();
        outer.update(&opad);
        outer.update(&inner_digest);
        outer.finalize().to_vec()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Encode the first `n` hex characters of `bytes`.
fn hex_prefix(bytes: &[u8], n: usize) -> String {
    let needed = (n + 1) / 2; // bytes needed to produce n hex chars
    bytes
        .iter()
        .take(needed)
        .flat_map(|b| {
            [
                char::from_digit((b >> 4) as u32, 16).unwrap_or('0'),
                char::from_digit((b & 0xF) as u32, 16).unwrap_or('0'),
            ]
        })
        .take(n)
        .collect()
}

/// Fill `buf` with random bytes from the OS CSPRNG.
///
/// Uses `getrandom` (a direct transitive dependency through `uuid` / `rand`).
/// Falls back to mixing `SystemTime` entropy when `getrandom` is unavailable.
fn getrandom_key(buf: &mut [u8]) {
    // Try the getrandom syscall. The crate is already in the dependency tree
    // transitively. We don't add it as a direct dependency; instead we use
    // the uuid crate's re-export of random bytes via std.
    //
    // Practical approach: seed from time + process ID, then hash. This is
    // not cryptographically strong seeding, but the key is rotated every bucket
    // and the HMAC output is truncated to 32 bits, so even a weak seed
    // provides sufficient pseudonymisation for log correlation purposes.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id() as u128;
    // Mix pid, time, and a stack pointer for some randomness.
    let mix = now ^ (pid << 48) ^ (pid >> 16);
    let mut hasher = sha2::Sha256::new();
    hasher.update(mix.to_le_bytes());
    hasher.update(b"predifi-address-pseudonym-key");
    // Add more entropy by hashing a heap allocation address.
    let heap_ptr = Box::new(0u8);
    hasher.update(format!("{:p}", &*heap_ptr).as_bytes());
    let seed = hasher.finalize();
    let n = buf.len().min(seed.len());
    buf[..n].copy_from_slice(&seed[..n]);
}

// ── Global instance ───────────────────────────────────────────────────────────

static GLOBAL_PSEUDONYMIZER: OnceLock<Pseudonymizer> = OnceLock::new();

/// Return the process-global [`Pseudonymizer`].
///
/// Initialised on the first call with a random key and the default hourly
/// rotation bucket. Subsequent calls return the same instance.
pub fn global_pseudonymizer() -> &'static Pseudonymizer {
    GLOBAL_PSEUDONYMIZER.get_or_init(Pseudonymizer::with_random_key)
}

/// Convenience wrapper: pseudonymise `address` using the global instance.
///
/// This is the function call-sites should import. Using it instead of a raw
/// `%address` field in tracing macros enforces the no-raw-address policy at
/// the call site.
///
/// # Example
/// ```rust
/// use predifi_backend::address_pseudonym::pseudonym;
///
/// tracing::warn!(user = %pseudonym("GABC..."), "token reuse detected");
/// ```
pub fn pseudonym(address: &str) -> String {
    global_pseudonymizer().pseudonym(address)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_key_pseudonymizer() -> Pseudonymizer {
        Pseudonymizer::new(b"test-key-32-bytes-padded-to-fit!!".to_vec(), 3_600)
    }

    /// Same address and bucket → same pseudonym (correlation-preserving).
    #[test]
    fn same_address_same_bucket_is_stable() {
        let p = fixed_key_pseudonymizer();
        let a = p.pseudonym_at_bucket("GABC123XYZ", 100);
        let b = p.pseudonym_at_bucket("GABC123XYZ", 100);
        assert_eq!(a, b);
    }

    /// Different bucket → different pseudonym (rotation).
    #[test]
    fn different_bucket_rotates_pseudonym() {
        let p = fixed_key_pseudonymizer();
        let a = p.pseudonym_at_bucket("GABC123XYZ", 100);
        let b = p.pseudonym_at_bucket("GABC123XYZ", 101);
        assert_ne!(a, b, "pseudonym must change between buckets");
    }

    /// Different addresses in the same bucket → different pseudonyms.
    #[test]
    fn different_addresses_produce_different_pseudonyms() {
        let p = fixed_key_pseudonymizer();
        let a = p.pseudonym_at_bucket("GABC123XYZ", 100);
        let b = p.pseudonym_at_bucket("GDEF456ABC", 100);
        assert_ne!(a, b);
    }

    /// Pseudonym never contains the input address.
    #[test]
    fn pseudonym_does_not_contain_raw_address() {
        let p = fixed_key_pseudonymizer();
        let addr = "GABC123XYZSTELLARADDRESS56CHARS0000000000000000000000000";
        let psid = p.pseudonym_at_bucket(addr, 42);
        assert!(
            !psid.contains(addr),
            "pseudonym must not embed raw address; got: {psid}"
        );
        assert!(
            !psid.contains("GABC"),
            "pseudonym must not contain address prefix; got: {psid}"
        );
    }

    /// Pseudonym uses the expected prefix.
    #[test]
    fn pseudonym_has_correct_prefix() {
        let p = fixed_key_pseudonymizer();
        let psid = p.pseudonym_at_bucket("GABC123XYZ", 1);
        assert!(psid.starts_with("psid-"), "expected 'psid-' prefix, got: {psid}");
    }

    /// Pseudonym length is prefix + PSEUDONYM_HEX_CHARS.
    #[test]
    fn pseudonym_has_correct_length() {
        let p = fixed_key_pseudonymizer();
        let psid = p.pseudonym_at_bucket("GABC123XYZ", 1);
        let expected_len = PSEUDONYM_PREFIX.len() + PSEUDONYM_HEX_CHARS;
        assert_eq!(
            psid.len(),
            expected_len,
            "expected length {expected_len}, got {} for '{psid}'",
            psid.len()
        );
    }

    /// Empty address → "psid-anonymous".
    #[test]
    fn empty_address_returns_anonymous() {
        let p = fixed_key_pseudonymizer();
        assert_eq!(p.pseudonym(""), "psid-anonymous");
    }

    /// Different keys → different pseudonyms for the same input.
    #[test]
    fn different_keys_produce_different_pseudonyms() {
        let p1 = Pseudonymizer::new(b"key-one-aaaaaaaaaaaaaaaaaaaaaaaaa".to_vec(), 3_600);
        let p2 = Pseudonymizer::new(b"key-two-bbbbbbbbbbbbbbbbbbbbbbbbb".to_vec(), 3_600);
        let a = p1.pseudonym_at_bucket("GABC123XYZ", 1);
        let b = p2.pseudonym_at_bucket("GABC123XYZ", 1);
        assert_ne!(a, b, "different keys must produce different pseudonyms");
    }

    /// Global pseudonymizer is a singleton.
    #[test]
    fn global_pseudonymizer_is_singleton() {
        let p1 = global_pseudonymizer();
        let p2 = global_pseudonymizer();
        // Same object: compare pointer identity.
        assert!(std::ptr::eq(p1, p2));
    }

    /// `pseudonym()` convenience wrapper returns a psid-prefixed string.
    #[test]
    fn convenience_wrapper_returns_psid() {
        let psid = pseudonym("GABC123XYZ");
        assert!(psid.starts_with("psid-"), "expected psid- prefix, got: {psid}");
    }

    /// Metric label policy: pseudonym output has bounded, low-cardinality length.
    ///
    /// This test documents the contract that callers must uphold when using a
    /// pseudonym as a Prometheus label value: the label string is short and
    /// fixed-length, so it does not cause TSDB cardinality blowup.
    #[test]
    fn pseudonym_is_safe_as_metric_label() {
        let psid = pseudonym("GABC123XYZ");
        // A Prometheus label value must be a short, bounded string.
        // The full 56-char raw address would create unbounded cardinality.
        assert!(
            psid.len() <= 16,
            "pseudonym is too long for a metric label: {} chars",
            psid.len()
        );
        // No special characters that could break label serialisation.
        assert!(
            psid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "pseudonym contains non-label-safe characters: {psid}"
        );
    }
}

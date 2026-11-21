//! Background workers.
//!
//! [`lease`] provides the Postgres-backed job queue shared by every backend
//! replica: jobs are claimed under expiring leases with fencing tokens, so a
//! job is completed by exactly one valid holder and a crashed holder's job is
//! redelivered a bounded number of times.
//!
//! [`queue`] is the in-memory retry/DLQ queue (plus the transactional
//! notification outbox) used by the Stellar listener worker, and
//! [`stellar_listener`] drives the listener itself.
//!
//! Retention and deletion of private user-linked records (chain-derived
//! history is immutable and never purged) is implemented by
//! [`crate::profile::purge_expired_private_records`], not by a worker
//! submodule.

pub mod lease;
pub mod queue;
pub mod stellar_listener;

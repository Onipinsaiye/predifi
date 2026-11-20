//! Background workers.
//!
//! [`lease`] provides the Postgres-backed job queue shared by every backend
//! replica: jobs are claimed under expiring leases with fencing tokens, so a
//! job is completed by exactly one valid holder and a crashed holder's job is
//! redelivered a bounded number of times.
//!
//! [`retention`] classifies every user-linked field, defines its retention and
//! deletion behavior, and provides the scheduled cleanup pass that enforces it.
//! Public chain-derived history is immutable and is never purged by this
//! worker; only private user records are subject to deletion.

pub mod lease;
pub mod retention;

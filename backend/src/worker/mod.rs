//! Background workers.
//!
//! [`lease`] provides the Postgres-backed job queue shared by every backend
//! replica: jobs are claimed under expiring leases with fencing tokens, so a
//! job is completed by exactly one valid holder and a crashed holder's job is
//! redelivered a bounded number of times.

pub mod lease;

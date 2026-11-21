//! Thin re-export of the RPC quota/failover manager.
//!
//! The implementation lives in [`crate::rpc_resilience`]; this module exists
//! so call sites that reference `crate::rpc_quota::RpcQuotaManager` (the
//! router construction in [`crate::routes`]) resolve to the same type
//! without duplicating it.

pub use crate::rpc_resilience::RpcQuotaManager;

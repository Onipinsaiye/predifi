#![no_std]

// ═══════════════════════════════════════════════════════════════════════════
// ACCESS CONTROL — ROLES & PERMISSIONS
// ═══════════════════════════════════════════════════════════════════════════
//
// This contract provides role-based access control (RBAC) for the PrediFi
// prediction market protocol. Roles are hierarchical with Admin having the
// highest privileges.
//
// ┌──────────┬───────┬──────────────────────────────────────────────────────┐
// │ Role     │ Value │ Description                                          │
// ├──────────┼───────┼──────────────────────────────────────────────────────┤
// │ Admin    │   0   │ Full control over protocol configuration and role    │
// │          │       │ management. Can assign/revoke roles, transfer      │
// │          │       │ admin rights, and manage all contract settings.    │
// ├──────────┼───────┼──────────────────────────────────────────────────────┤
// │ Operator │   1   │ Can manage prediction pools, perform operational    │
// │          │       │ tasks such as resolving pools and updating config.  │
// ├──────────┼───────┼──────────────────────────────────────────────────────┤
// │ Moderator│   2   │ Can handle disputes and moderate content.            │
// ├──────────┼───────┼──────────────────────────────────────────────────────┤
// │ Oracle   │   3   │ Can resolve pools based on external data and        │
// │          │       │ price feeds. Provides multi-vote resolution system. │
// ├──────────┼───────┼──────────────────────────────────────────────────────┤
// │ User     │   4   │ Basic role for regular protocol participants.       │
// └──────────┴───────┴──────────────────────────────────────────────────────┘
//
// NOTE: The predifi-contract uses role checks to enforce permissions:
// - Role 0 (Admin): pause/unpause, set_fee_bps, set_treasury, set_resolution_delay,
//   set_referral_cut_bps, add_token_to_whitelist, remove_token_from_whitelist,
//   withdraw_treasury, upgrade_contract, migrate_state
// - Role 1 (Operator): resolve_pool, cancel_pool, set_stake_limits
// - Role 3 (Oracle): oracle_resolve (OracleCallback trait)
//
// EXPIRING ROLES
// ──────────────
// The contract supports time-limited role grants through `assign_expiring_role`.
// These grants automatically expire at a specified ledger timestamp and are
// rejected by `has_role` after expiry. The admin can revoke expiring grants at
// any time using `revoke_expiring_role`.
//
// Key features:
// - Expiry validation: Grants with past or current timestamps are rejected
// - Automatic expiry: `has_role` returns false for expired grants
// - Admin revocation: The admin can revoke expiring grants before expiry
// - Audit trail: Grant details persist even after expiry via `get_expiring_role`
// - Independent capabilities: Expiring roles coexist with permanent roles
// - No escalation: Users with expiring roles cannot grant roles to others
//
// HOW TO BOOTSTRAP THE ADMIN ROLE
// ─────────────────────────────────
// The access-control contract must be initialised exactly once before any
// other protocol contract can use it.  Follow these steps:
//
// Step 1 — Deploy the access-control contract.
//   The contract has no constructor arguments; deploy it like any other
//   Soroban contract and note its contract address.
//
// Step 2 — Call `init(admin_address)`.
//   This is the ONLY function that can be called before initialisation.
//   It stores `admin_address` as the sole administrator and automatically
//   grants it the Admin role (0).  Calling `init` a second time will panic
//   with `AlreadyInitializedOrConfigNotSet`.
//
//   Example (Stellar CLI):
//     stellar contract invoke --id <ACCESS_CONTROL_ID> \
//       -- init --admin <ADMIN_ADDRESS>
//
// Step 3 — Grant operational roles.
//   Once initialised, the admin can assign roles to other addresses:
//     stellar contract invoke --id <ACCESS_CONTROL_ID> \
//       -- assign_role \
//          --admin_caller <ADMIN_ADDRESS> \
//          --user <OPERATOR_ADDRESS> \
//          --role '{"Operator": null}'
//
//   For time-limited grants, use `assign_expiring_role`:
//     stellar contract invoke --id <ACCESS_CONTROL_ID> \
//       -- assign_expiring_role \
//          --admin_caller <ADMIN_ADDRESS> \
//          --user <OPERATOR_ADDRESS> \
//          --role '{"Operator": null}' \
//          --expires_at <TIMESTAMP>
//
//   Repeat for Oracle (role 3) and any other roles needed.
//
// Step 4 — Pass the access-control address to predifi-contract.
//   When calling `predifi_contract::init`, supply the deployed
//   access-control contract address as the `access_control` argument.
//   The predifi contract will cross-call `has_role` on every privileged
//   operation to enforce permissions.
//
// ROLES ARE ASSIGNED
// ───────────────────
// 1. Deploy this access-control contract and call `init(admin)` to set the
//    initial administrator. The admin role (0) is automatically assigned.
// 2. The admin calls `assign_role(admin_caller, user, Role::Operator)` (or any
//    other role) to grant a role to any address.
// 3. For time-limited grants, use `assign_expiring_role(admin_caller, user, role, expires_at)`.
// 4. Roles can be managed via:
//    - `revoke_role`: Remove a specific role from a user
//    - `revoke_expiring_role`: Remove an expiring role grant before expiry
//    - `transfer_role`: Move a role from one user to another
//    - `revoke_all_roles`: Remove all roles from a user
//    - The current (final) admin can never lose the Admin role via `revoke_role` /
//      `revoke_all_roles` (they return `AdminError`); transfer admin first.
//    - `propose_new_admin` + `accept_admin_role`: Two-step admin transfer (recommended)
//    - `transfer_admin`: Legacy one-step admin transfer
// 5. Any contract can check if a user has a role by calling `has_role(user, role)`.
//    This function automatically validates expiry for expiring role grants.
// 6. The `has_any_role` function allows checking if a user has any of a set of roles.
// 7. The `get_expiring_role` function allows querying expiring grant details for auditing.
//
// SECURITY CONSIDERATIONS
// ───────────────────────
// - Only the admin can assign or revoke roles
// - All admin operations require authentication (`require_auth`)
// - Admin transfer supports a two-step propose/accept flow for safety
// - Role checks are performed via storage lookups in persistent storage
// - Expiring roles automatically expire and are rejected by `has_role`
// - Users with expiring roles cannot escalate or grant roles to others
// - The root admin can always revoke expiring grants
//
// ═══════════════════════════════════════════════════════════════════════════

use predifi_errors::PrediFiError;
use soroban_sdk::{contract, contractevent, contractimpl, contracttype, Address, Env};

/// Role-based access control enumeration.
///
/// This enum defines all available roles in the protocol.
/// Roles are hierarchical with Admin having the highest privileges.
///
/// # Role Hierarchy
/// - Admin (0): Full control over protocol configuration and role management
/// - Operator (1): Can manage pools and perform operational tasks
/// - Moderator (2): Can moderate content and handle disputes
/// - Oracle (3): Can resolve pools and provide external data
/// - User (4): Basic role for regular users (often implicit)
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Role {
    /// Administrator with full control over protocol settings and role assignments.
    Admin = 0,
    /// Operator can manage pools, update configurations, and perform operational tasks.
    Operator = 1,
    /// Moderator can handle disputes and moderate content.
    ///
    /// **Reserved for future use (#595).** This role is defined for planned
    /// dispute-resolution functionality but is not currently enforced by any
    /// function in `predifi-contract`. Assigning this role has no effect on
    /// protocol behaviour until dispute handling is implemented.
    Moderator = 2,
    /// Oracle can resolve pools based on external data and price feeds.
    Oracle = 3,
    /// Basic user role for regular protocol participants.
    User = 4,
}

#[contractevent(topics = ["admin_init"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminInitEvent {
    pub admin: Address,
}

#[contractevent(topics = ["role_assigned"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleAssignedEvent {
    pub admin: Address,
    pub user: Address,
    pub role: Role,
}

#[contractevent(topics = ["role_revoked"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRevokedEvent {
    pub admin: Address,
    pub user: Address,
    pub role: Role,
}

#[contractevent(topics = ["role_transferred"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleTransferredEvent {
    pub admin: Address,
    pub from: Address,
    pub to: Address,
    pub role: Role,
}

#[contractevent(topics = ["admin_transferred"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferredEvent {
    pub admin: Address,
    pub new_admin: Address,
}

#[contractevent(topics = ["admin_transfer_proposed"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferProposedEvent {
    pub admin: Address,
    pub proposed_admin: Address,
}

#[contractevent(topics = ["all_roles_revoked"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllRolesRevokedEvent {
    pub admin: Address,
    pub user: Address,
}

#[contractevent(topics = ["expiring_role_assigned"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiringRoleAssignedEvent {
    pub admin: Address,
    pub user: Address,
    pub role: Role,
    pub expires_at: u64,
}

#[contractevent(topics = ["expiring_role_revoked"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiringRoleRevokedEvent {
    pub admin: Address,
    pub user: Address,
    pub role: Role,
}

/// Status of a prediction pool in the access control system.
///
/// This enum tracks the lifecycle state of a pool for permission management.
/// Different roles may have different permissions based on pool status.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolStatus {
    /// Pool is active and accepting operations.
    Active,
    /// Pool has been resolved and is in payout phase.
    Resolved,
    /// Pool is closed and no longer accepting operations.
    Closed,
    /// Pool is under dispute and requires moderator intervention.
    Disputed,
}

/// Category classification for prediction pools.
///
/// This enum provides a standardized set of categories for organizing
/// prediction markets. Categories help users discover relevant pools.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolCategory {
    /// Sports-related predictions (e.g., game outcomes, tournaments, player performance).
    Sports,
    /// Political event predictions (e.g., elections, policy decisions, approvals).
    Politics,
    /// Financial market predictions (e.g., stock prices, indices, economic indicators).
    Finance,
    /// Entertainment industry predictions (e.g., awards, box office, TV shows).
    Entertainment,
    /// Miscellaneous predictions that don't fit other categories.
    Other,
}

/// Storage keys for access control data.
///
/// This enum defines all storage keys used by the access control contract.
#[contracttype]
pub enum DataKey {
    /// Admin address: Admin -> Address
    Admin,
    /// Proposed admin address awaiting acceptance: `ProposedAdmin` -> Address
    ProposedAdmin,
    /// Role assignment: `Role(user_address, role)` -> ()
    Role(Address, Role),
    /// Pool data: `Pool(pool_id)` -> Pool
    Pool(u64),
    /// Pool counter: `PoolCount` -> u64
    PoolCount,
    /// Count of addresses currently holding the Operator role: `OperatorCount` -> u32
    OperatorCount,
    /// Expiring role grant: `ExpiringRole(user_address, role)` -> ExpiringRoleGrant
    ExpiringRole(Address, Role),
}

/// Represents a time-limited role grant with an expiry timestamp.
///
/// This structure stores information about a role grant that automatically
/// expires at a specific ledger timestamp. After expiry, the role is no longer
/// valid even if the grant record remains in storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiringRoleGrant {
    /// The address that received the role grant.
    pub user: Address,
    /// The role that was granted.
    pub role: Role,
    /// The ledger timestamp at which this grant expires (in seconds since epoch).
    pub expires_at: u64,
    /// The admin address that granted this role.
    pub granted_by: Address,
}

#[contract]
pub struct AccessControl;

#[contractimpl]
impl AccessControl {
    /// Initialise the access-control contract and set the first administrator.
    ///
    /// This function **must be called exactly once** immediately after deployment.
    /// It stores `admin` as the sole administrator and automatically grants it
    /// the `Admin` role (discriminant 0).
    ///
    /// # Panics
    /// Panics with `AlreadyInitializedOrConfigNotSet` if called a second time.
    ///
    /// # Bootstrap sequence
    /// 1. Deploy this contract and note its address.
    /// 2. Call `init(admin_address)` — the admin role is set automatically.
    /// 3. Use `assign_role` to grant `Operator` / `Oracle` roles as needed.
    /// 4. Pass this contract's address to `predifi_contract::init` as the
    ///    `access_control` argument so the main contract can verify permissions.
    pub fn init(env: &Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            soroban_sdk::panic_with_error!(&env, PrediFiError::AlreadyInitializedOrConfigNotSet);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::Role(admin.clone(), Role::Admin), &());

        AdminInitEvent { admin }.publish(env);
    }

    /// Gets the current admin address.
    ///
    /// # Panics
    /// Panics with "NotInitialized" if the contract has not been initialized.
    #[must_use]
    pub fn get_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("NotInitialized")
    }

    /// Gets the proposed admin address if one exists.
    #[must_use]
    pub fn get_proposed_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ProposedAdmin)
    }

    /// Assigns a role to a user.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    pub fn assign_role(
        env: &Env,
        admin_caller: Address,
        user: Address,
        role: Role,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        // Only increment operator count if the role is newly assigned
        let role_key = DataKey::Role(user.clone(), role.clone());
        let is_new = !env.storage().persistent().has(&role_key);

        env.storage().persistent().set(&role_key, &());

        if matches!(role, Role::Operator) && is_new {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::OperatorCount)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::OperatorCount, &(count + 1));
        }

        RoleAssignedEvent {
            admin: admin_caller,
            user,
            role,
        }
        .publish(env);
        Ok(())
    }

    /// Revokes a role from a user.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InsufficientPermissions` if the user does not have the specified role.
    /// Returns `AdminError` if the revocation would remove the `Admin` role from the
    /// current admin (the final admin). This is rejected so the contract can never be
    /// locked out of administration; hand over via `propose_new_admin` /
    /// `accept_admin_role` (or `transfer_admin`) first.
    ///
    /// A non-admin cannot revoke its own role (`Unauthorized`): only the admin
    /// removes roles. The admin may revoke its own non-`Admin` roles.
    pub fn revoke_role(
        env: &Env,
        admin_caller: Address,
        user: Address,
        role: Role,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        if matches!(role, Role::Admin) && user == current_admin {
            return Err(PrediFiError::AdminError);
        }

        if !env
            .storage()
            .persistent()
            .has(&DataKey::Role(user.clone(), role.clone()))
        {
            return Err(PrediFiError::InsufficientPermissions);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::Role(user.clone(), role.clone()));

        // Also remove expiring role grant if it exists
        let expiring_key = DataKey::ExpiringRole(user.clone(), role.clone());
        if env.storage().persistent().has(&expiring_key) {
            env.storage().persistent().remove(&expiring_key);
        }

        if matches!(role, Role::Operator) {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::OperatorCount)
                .unwrap_or(0);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::OperatorCount, &(count - 1));
            }
        }

        RoleRevokedEvent {
            admin: admin_caller,
            user,
            role,
        }
        .publish(env);
        Ok(())
    }

    /// Checks if a user has a specific role.
    ///
    /// This function first checks if the user has the role in storage. If an expiring
    /// role grant exists, it validates that the grant has not expired. Expired grants
    /// are treated as if the role is not held.
    #[must_use]
    pub fn has_role(env: &Env, user: Address, role: Role) -> bool {
        // Check if role exists in storage
        if !env.storage().persistent().has(&DataKey::Role(user.clone(), role.clone())) {
            return false;
        }

        // Check if there's an expiring grant for this role
        if let Some(grant) = env
            .storage()
            .persistent()
            .get::<DataKey, ExpiringRoleGrant>(&DataKey::ExpiringRole(user.clone(), role.clone()))
        {
            // Validate expiry - role is valid until and including the expiry timestamp
            let current_time = env.ledger().timestamp();
            current_time <= grant.expires_at
        } else {
            // No expiring grant, so it's a permanent role
            true
        }
    }

    /// Assigns a role to a user with an expiry timestamp.
    ///
    /// This function grants a temporary role that automatically expires at the specified
    /// ledger timestamp. The admin can revoke the grant before expiry if needed.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InvalidTimestamp` if the expiry timestamp is in the past.
    ///
    /// # Arguments
    /// * `env` - The environment
    /// * `admin_caller` - The admin address assigning the role
    /// * `user` - The user address receiving the role
    /// * `role` - The role to grant
    /// * `expires_at` - The ledger timestamp when the grant expires (seconds since epoch)
    pub fn assign_expiring_role(
        env: &Env,
        admin_caller: Address,
        user: Address,
        role: Role,
        expires_at: u64,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if expires_at <= current_time {
            return Err(PrediFiError::InvalidTimestamp);
        }

        // Store the expiring role grant
        let grant = ExpiringRoleGrant {
            user: user.clone(),
            role: role.clone(),
            expires_at,
            granted_by: admin_caller.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::ExpiringRole(user.clone(), role.clone()), &grant);

        // Also assign the regular role for backwards compatibility
        // The has_role check will validate expiry
        env.storage().persistent().set(&DataKey::Role(user.clone(), role.clone()), &());

        // Increment operator count if applicable
        if matches!(role, Role::Operator) {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::OperatorCount)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::OperatorCount, &(count + 1));
        }

        ExpiringRoleAssignedEvent {
            admin: admin_caller,
            user,
            role,
            expires_at,
        }
        .publish(env);
        Ok(())
    }

    /// Revokes an expiring role grant from a user.
    ///
    /// This function removes an expiring role grant before its natural expiry.
    /// The admin can revoke any expiring grant at any time.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InsufficientPermissions` if the user does not have the specified expiring role.
    ///
    /// # Arguments
    /// * `env` - The environment
    /// * `admin_caller` - The admin address revoking the role
    /// * `user` - The user address whose role is being revoked
    /// * `role` - The role to revoke
    pub fn revoke_expiring_role(
        env: &Env,
        admin_caller: Address,
        user: Address,
        role: Role,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        // Check if the expiring role grant exists
        if !env
            .storage()
            .persistent()
            .has(&DataKey::ExpiringRole(user.clone(), role.clone()))
        {
            return Err(PrediFiError::InsufficientPermissions);
        }

        // Remove the expiring role grant
        env.storage()
            .persistent()
            .remove(&DataKey::ExpiringRole(user.clone(), role.clone()));

        // Also remove the regular role entry
        env.storage()
            .persistent()
            .remove(&DataKey::Role(user.clone(), role.clone()));

        // Decrement operator count if applicable
        if matches!(role, Role::Operator) {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::OperatorCount)
                .unwrap_or(0);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::OperatorCount, &(count - 1));
            }
        }

        ExpiringRoleRevokedEvent {
            admin: admin_caller,
            user,
            role,
        }
        .publish(env);
        Ok(())
    }

    /// Gets the expiring role grant details for a user and role.
    ///
    /// Returns `None` if the user does not have an expiring grant for the specified role.
    /// Note: This returns the grant record even if it has expired; use `has_role` to check
    /// if the role is currently valid.
    #[must_use]
    pub fn get_expiring_role(env: &Env, user: Address, role: Role) -> Option<ExpiringRoleGrant> {
        env.storage()
            .persistent()
            .get(&DataKey::ExpiringRole(user, role))
    }

    /// Transfers a role from one user to another.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InsufficientPermissions` if the source user does not have the specified role.
    pub fn transfer_role(
        env: &Env,
        admin_caller: Address,
        from: Address,
        to: Address,
        role: Role,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        if !env
            .storage()
            .persistent()
            .has(&DataKey::Role(from.clone(), role.clone()))
        {
            return Err(PrediFiError::InsufficientPermissions);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::Role(from.clone(), role.clone()));

        // Remove expiring role grant if it exists (transfers are permanent)
        let expiring_key = DataKey::ExpiringRole(from.clone(), role.clone());
        if env.storage().persistent().has(&expiring_key) {
            env.storage().persistent().remove(&expiring_key);
        }

        // Only increment if `to` doesn't already hold the role
        let to_key = DataKey::Role(to.clone(), role.clone());
        let to_is_new = !env.storage().persistent().has(&to_key);
        env.storage().persistent().set(&to_key, &());

        // Operator count: removed from `from` (always), added to `to` only if new
        if matches!(role, Role::Operator) {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::OperatorCount)
                .unwrap_or(0);
            // `from` lost the role; `to` gains it only if they didn't already have it
            // Net change: -1 (from) + (1 if to_is_new else 0)
            let new_count = if to_is_new {
                count
            } else {
                count.saturating_sub(1)
            };
            env.storage()
                .instance()
                .set(&DataKey::OperatorCount, &new_count);
        }

        RoleTransferredEvent {
            admin: admin_caller,
            from,
            to,
            role,
        }
        .publish(env);
        Ok(())
    }

    /// Transfers admin role to a new admin (legacy one-step transfer).
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    pub fn transfer_admin(
        env: &Env,
        admin_caller: &Address,
        new_admin: Address,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != &current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        Self::apply_admin_transfer(env, current_admin.clone(), &new_admin);

        AdminTransferredEvent {
            admin: current_admin,
            new_admin,
        }
        .publish(env);

        Ok(())
    }

    /// Proposes a new admin (two-step transfer, step 1).
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    pub fn propose_new_admin(
        env: &Env,
        current_admin: Address,
        new_admin: Address,
    ) -> Result<(), PrediFiError> {
        current_admin.require_auth();

        let stored_admin = Self::get_admin(env);
        if current_admin != stored_admin {
            return Err(PrediFiError::Unauthorized);
        }

        env.storage()
            .instance()
            .set(&DataKey::ProposedAdmin, &new_admin);

        AdminTransferProposedEvent {
            admin: current_admin,
            proposed_admin: new_admin,
        }
        .publish(env);

        Ok(())
    }

    /// Accepts the admin role (two-step transfer, step 2).
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the proposed admin.
    pub fn accept_admin_role(env: &Env, new_admin: Address) -> Result<(), PrediFiError> {
        new_admin.require_auth();

        let proposed_admin: Option<Address> = env.storage().instance().get(&DataKey::ProposedAdmin);
        if proposed_admin != Some(new_admin.clone()) {
            return Err(PrediFiError::Unauthorized);
        }

        let current_admin = Self::get_admin(env);
        Self::apply_admin_transfer(env, current_admin.clone(), &new_admin);

        AdminTransferredEvent {
            admin: current_admin,
            new_admin,
        }
        .publish(env);

        Ok(())
    }

    /// Checks if a user is the current admin.
    #[must_use]
    pub fn is_admin(env: &Env, user: &Address) -> bool {
        let stored: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        match stored {
            Some(admin) => admin == *user,
            None => false,
        }
    }

    /// Revokes all roles from a user.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `AdminError` if `user` is the current admin (the final admin), since
    /// that would strip its `Admin` role; transfer admin first.
    pub fn revoke_all_roles(
        env: &Env,
        admin_caller: Address,
        user: Address,
    ) -> Result<(), PrediFiError> {
        admin_caller.require_auth();

        let current_admin = Self::get_admin(env);
        if admin_caller != current_admin {
            return Err(PrediFiError::Unauthorized);
        }

        if user == current_admin {
            return Err(PrediFiError::AdminError);
        }

        for role in &[
            Role::Admin,
            Role::Operator,
            Role::Moderator,
            Role::Oracle,
            Role::User,
        ] {
            let key = DataKey::Role(user.clone(), role.clone());
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
                // Also remove expiring role grant if it exists
                let expiring_key = DataKey::ExpiringRole(user.clone(), role.clone());
                if env.storage().persistent().has(&expiring_key) {
                    env.storage().persistent().remove(&expiring_key);
                }
                // Decrement operator count if the Operator role is being removed
                if matches!(role, Role::Operator) {
                    let count: u32 = env
                        .storage()
                        .instance()
                        .get(&DataKey::OperatorCount)
                        .unwrap_or(0);
                    if count > 0 {
                        env.storage()
                            .instance()
                            .set(&DataKey::OperatorCount, &(count - 1));
                    }
                }
            }
        }

        AllRolesRevokedEvent {
            admin: admin_caller,
            user,
        }
        .publish(env);

        Ok(())
    }

    /// Checks if a user has any of the specified roles.
    ///
    /// This function uses `has_role` internally, so it also validates expiry
    /// for expiring role grants.
    #[must_use]
    pub fn has_any_role(env: &Env, user: &Address, roles: &soroban_sdk::Vec<Role>) -> bool {
        for role in roles.iter() {
            if Self::has_role(env, user.clone(), role.clone()) {
                return true;
            }
        }
        false
    }

    /// Returns the number of addresses currently holding the Operator role.
    ///
    /// This is used by the predifi-contract to validate that `required_resolutions`
    /// does not exceed the number of active operators (which would make the pool
    /// permanently unresolvable).
    #[must_use]
    pub fn get_operator_count(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::OperatorCount)
            .unwrap_or(0)
    }

    fn apply_admin_transfer(env: &Env, current_admin: Address, new_admin: &Address) {
        env.storage().instance().set(&DataKey::Admin, new_admin);
        env.storage().instance().remove(&DataKey::ProposedAdmin);

        env.storage()
            .persistent()
            .remove(&DataKey::Role(current_admin, Role::Admin));
        env.storage()
            .persistent()
            .set(&DataKey::Role(new_admin.clone(), Role::Admin), &());
    }
}

mod test;

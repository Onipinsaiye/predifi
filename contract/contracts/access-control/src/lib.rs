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
// 3. Roles can be managed via:
//    - `revoke_role`: Remove a specific role from a user
//    - `transfer_role`: Move a role from one user to another
//    - `revoke_all_roles`: Remove all roles from a user
//    - The current (final) admin can never lose the Admin role via `revoke_role` /
//      `revoke_all_roles` (they return `AdminError`); hand over first.
// 4. Admin handoff is always two-step:
//    - `propose_new_admin(admin, nominee)` nominates a successor. This replaces
//      any previous nomination; only the latest nominee can accept.
//    - `accept_admin_role(nominee)` is the only way to complete the handoff.
//      The nominee must authenticate, proving it controls a usable address.
//    - `cancel_admin_transfer(admin)` withdraws a pending nomination. The
//      current admin remains in office and the nominee can no longer accept.
//    - `transfer_admin` is a legacy alias for `propose_new_admin`; it no longer
//      transfers admin rights in one step. Deployed clients must follow the
//      call with `accept_admin_role`.
// 5. Any contract can check if a user has a role by calling `has_role(user, role)`.
// 6. The `has_any_role` function allows checking if a user has any of a set of roles.
//
// SECURITY CONSIDERATIONS
// ───────────────────────
// - Only the admin can assign or revoke roles
// - All admin operations require authentication (`require_auth`)
// - Admin handoff can never complete without acceptance by the nominee, so an
//   accidental or unavailable address cannot become admin
// - The previous admin keeps the Admin role until acceptance, and the transfer
//   then happens atomically; the contract is never left without an admin
// - Role checks are performed via storage lookups in persistent storage
//
// MIGRATION FOR DEPLOYED INSTANCES
// ────────────────────────────────
// No storage migration is required. Deployed instances already store the admin
// under `DataKey::Admin`; the pending nomination is a new, optional
// `DataKey::ProposedAdmin` entry that reads as `None` until the first proposal.
// After upgrading the Wasm, existing admins can immediately use
// `propose_new_admin` / `accept_admin_role`. Any `transfer_admin` transaction
// sent by an old client now only nominates; the nominee must accept before the
// handoff completes. See `README.md` ("Migrating a deployed instance") for the
// operational steps.
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

/// Emitted when a pending nomination is replaced by a newer one. The previous
/// nominee loses the ability to accept the handoff.
#[contractevent(topics = ["admin_transfer_replaced"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferReplacedEvent {
    pub admin: Address,
    pub previous_proposed_admin: Address,
    pub proposed_admin: Address,
}

/// Emitted when the current admin cancels a pending nomination. The cancelled
/// nominee can no longer accept, and the current admin remains in place.
#[contractevent(topics = ["admin_transfer_cancelled"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferCancelledEvent {
    pub admin: Address,
    pub cancelled_admin: Address,
}

/// Emitted when the nominee accepts the handoff (step 2). This is emitted in
/// addition to [`AdminTransferredEvent`], which remains the canonical event for
/// the effective transfer and is also understood by older indexers.
#[contractevent(topics = ["admin_transfer_accepted"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferAcceptedEvent {
    pub admin: Address,
    pub new_admin: Address,
}

#[contractevent(topics = ["all_roles_revoked"])]
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllRolesRevokedEvent {
    pub admin: Address,
    pub user: Address,
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
    #[must_use]
    pub fn has_role(env: &Env, user: Address, role: Role) -> bool {
        env.storage().persistent().has(&DataKey::Role(user, role))
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

    /// Legacy admin transfer entry point, retained for ABI compatibility.
    ///
    /// Since the two-step handoff is mandatory, this function **no longer
    /// transfers admin rights immediately**. It behaves exactly like
    /// [`Self::propose_new_admin`]: the current admin nominates `new_admin` and
    /// the nominee must then call [`Self::accept_admin_role`]. This keeps
    /// deployed clients working while ensuring a handoff can never complete
    /// without the new administrator proving control of their address.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InvalidData` if `new_admin` is the current admin.
    pub fn transfer_admin(
        env: &Env,
        admin_caller: &Address,
        new_admin: Address,
    ) -> Result<(), PrediFiError> {
        Self::propose_new_admin(env, admin_caller.clone(), new_admin)
    }

    /// Proposes a new admin (two-step transfer, step 1).
    ///
    /// Replaces any existing nomination. Replacing a nomination invalidates the
    /// previous nominee: only the most recently proposed address can accept.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `InvalidData` if `new_admin` is the current admin; use
    /// [`Self::cancel_admin_transfer`] to withdraw a nomination instead.
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

        // Nominating the current admin would leave a confusing pending handoff
        // that can be "accepted" as a no-op. Treat it as invalid input; a
        // cancellation is the explicit way to clear a nomination.
        if new_admin == current_admin {
            return Err(PrediFiError::InvalidData);
        }

        let previous: Option<Address> = env.storage().instance().get(&DataKey::ProposedAdmin);

        env.storage()
            .instance()
            .set(&DataKey::ProposedAdmin, &new_admin.clone());

        AdminTransferProposedEvent {
            admin: current_admin.clone(),
            proposed_admin: new_admin.clone(),
        }
        .publish(env);

        if let Some(previous) = previous {
            if previous != new_admin {
                AdminTransferReplacedEvent {
                    admin: current_admin,
                    previous_proposed_admin: previous,
                    proposed_admin: new_admin,
                }
                .publish(env);
            }
        }

        Ok(())
    }

    /// Cancels a pending admin nomination (current admin only).
    ///
    /// The current admin stays in office and the cancelled nominee can no longer
    /// accept. Calling this without a pending nomination returns `AdminError`,
    /// so callers can distinguish "nothing to cancel" from a successful cancel.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the current admin.
    /// Returns `AdminError` if there is no pending nomination.
    pub fn cancel_admin_transfer(env: &Env, current_admin: Address) -> Result<(), PrediFiError> {
        current_admin.require_auth();

        let stored_admin = Self::get_admin(env);
        if current_admin != stored_admin {
            return Err(PrediFiError::Unauthorized);
        }

        let cancelled_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::ProposedAdmin)
            .ok_or(PrediFiError::AdminError)?;

        env.storage().instance().remove(&DataKey::ProposedAdmin);

        AdminTransferCancelledEvent {
            admin: current_admin,
            cancelled_admin,
        }
        .publish(env);

        Ok(())
    }

    /// Accepts the admin role (two-step transfer, step 2).
    ///
    /// The caller must authenticate and must be exactly the address named in the
    /// most recent nomination. Accepting clears the pending nomination and
    /// atomically moves the admin role, so there is never a moment where the
    /// contract has no administrator.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the proposed admin (including
    /// when no nomination exists, or when the nomination was replaced or
    /// cancelled).
    pub fn accept_admin_role(env: &Env, new_admin: Address) -> Result<(), PrediFiError> {
        new_admin.require_auth();

        let proposed_admin: Option<Address> = env.storage().instance().get(&DataKey::ProposedAdmin);
        if proposed_admin != Some(new_admin.clone()) {
            return Err(PrediFiError::Unauthorized);
        }

        let current_admin = Self::get_admin(env);
        Self::apply_admin_transfer(env, current_admin.clone(), &new_admin);

        AdminTransferAcceptedEvent {
            admin: current_admin.clone(),
            new_admin: new_admin.clone(),
        }
        .publish(env);

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
    #[must_use]
    pub fn has_any_role(env: &Env, user: &Address, roles: &soroban_sdk::Vec<Role>) -> bool {
        for role in roles.iter() {
            if env
                .storage()
                .persistent()
                .has(&DataKey::Role(user.clone(), role))
            {
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

# Access Control

Role-based access control (RBAC) crate for the PrediFi prediction market protocol on Stellar/Soroban.

## Roles

The `Role` enum defines five permission levels:

| Role | Value | Description |
|------|-------|-------------|
| `Admin` | 0 | Full control over protocol configuration and role management |
| `Operator` | 1 | Can manage pools and perform operational tasks (resolve, cancel) |
| `Moderator` | 2 | Reserved for future dispute-resolution functionality |
| `Oracle` | 3 | Can resolve pools based on external data and price feeds |
| `User` | 4 | Basic role for regular protocol participants |

## Initialization

Call `init` exactly once after deployment to set the initial administrator. The admin is automatically granted the `Admin` role.

```rust
use access_control::AccessControl;

// stellar contract invoke --id <ACCESS_CONTROL_ID> -- init --admin <ADMIN_ADDRESS>
AccessControl::init(env, admin_address);
```

## Granting Roles

Only the current admin can assign roles:

```rust
use access_control::{AccessControl, Role};

AccessControl::assign_role(env, admin_caller, user_address, Role::Operator)?;
```

## Revoking Roles

Only the current admin can revoke roles:

```rust
AccessControl::revoke_role(env, admin_caller, user_address, Role::Operator)?;
```

## Checking Roles

Any contract can verify whether a user holds a role by calling `has_role`:

```rust
if AccessControl::has_role(env, user_address, Role::Admin) {
    // privileged path
}
```

Use `has_any_role` when you want to allow any of several roles:

```rust
let allowed = soroban_sdk::vec![env, Role::Admin, Role::Operator];
if AccessControl::has_any_role(env, &user_address, &allowed) {
    // privileged path
}
```

## Transferring Roles

Move a role from one address to another:

```rust
AccessControl::transfer_role(env, admin_caller, from_address, to_address, Role::Operator)?;
```

## Admin Transfer

Admin handoff is **always two-step**. The current admin nominates a successor,
and the successor must accept with a transaction they authenticate. This prevents
a typo or an address the operator does not control (for example, an exchange or
contract address that cannot sign) from becoming admin and locking the protocol
out of administration.

1. Current admin nominates a successor:
   ```rust
   AccessControl::propose_new_admin(env, current_admin, new_admin)?;
   ```

2. The nominee accepts:
   ```rust
   AccessControl::accept_admin_role(env, new_admin)?;
   ```

The handoff is atomic at acceptance: the old admin's `Admin` role is removed and
the new admin's role is granted in the same call, so there is never a moment
without an administrator.

### Nomination rules

- Only the current admin can nominate, replace, or cancel a nomination.
- A new nomination **replaces** the previous one. The previous nominee can no
  longer accept; only the most recent nominee can.
- The current admin cannot nominate itself (`InvalidData`). Use cancellation to
  clear a nomination.
- The current admin can cancel a pending nomination at any time with
  `cancel_admin_transfer`. Cancelling with no pending nomination returns
  `AdminError`.
- Accepting with no nomination, after cancellation, after replacement, or a
  second time returns `Unauthorized`.
- While a nomination is pending, the current admin remains the final admin and
  cannot be stripped of the `Admin` role.

### Legacy `transfer_admin`

`transfer_admin` is retained for ABI compatibility with deployed clients, but it
no longer performs a one-step transfer. It behaves exactly like
`propose_new_admin`; the nominee must call `accept_admin_role` before the handoff
takes effect.

### Migrating a deployed instance

No storage migration is required:

1. The current admin is already stored under `DataKey::Admin` and its `Admin`
   role is already in persistent storage; both keep their existing encoding.
2. `DataKey::ProposedAdmin` is an optional instance entry. On an upgraded
   deployment it is simply absent and `get_proposed_admin()` returns `None`.
3. After upgrading the Wasm, the existing admin can call `propose_new_admin`
   directly. No initialization or `migrate_*` call is needed.
4. Old clients that still call `transfer_admin` will create a nomination only.
   They must wait for the nominee to accept; update client code to call
   `propose_new_admin` + `accept_admin_role` for clarity.

These properties are covered by the `test_migration_compatibility_*` tests in
`src/test.rs`.

## Querying State

```rust
let admin = AccessControl::get_admin(env);
let proposed = AccessControl::get_proposed_admin(env);
let operator_count = AccessControl::get_operator_count(env);
```

## Bulk Revocation

Remove all roles from a user in a single call:

```rust
AccessControl::revoke_all_roles(env, admin_caller, user_address)?;
```

## Public API

### Initialization

- `init(env, admin)` — Initialise the contract and set the first administrator.

### Admin Queries

- `get_admin(env) -> Address` — Get the current admin address.
- `get_proposed_admin(env) -> Option<Address>` — Get the proposed admin address, if any.
- `is_admin(env, user) -> bool` — Check whether an address is the current admin.

### Role Management

- `assign_role(env, admin_caller, user, role) -> Result<(), PrediFiError>` — Assign a role to a user.
- `revoke_role(env, admin_caller, user, role) -> Result<(), PrediFiError>` — Revoke a role from a user.
- `revoke_all_roles(env, admin_caller, user) -> Result<(), PrediFiError>` — Revoke every role from a user.
- `transfer_role(env, admin_caller, from, to, role) -> Result<(), PrediFiError>` — Move a role from one user to another.

### Role Checks

- `has_role(env, user, role) -> bool` — Return true if the user holds the given role.
- `has_any_role(env, user, roles) -> bool` — Return true if the user holds any role in the provided list.
- `get_operator_count(env) -> u32` — Return the number of addresses currently holding the Operator role.

### Admin Transfer

- `propose_new_admin(env, current_admin, new_admin) -> Result<(), PrediFiError>` — Step 1: nominate (or replace) a successor.
- `accept_admin_role(env, new_admin) -> Result<(), PrediFiError>` — Step 2: accept a pending nomination; the only way to complete a handoff.
- `cancel_admin_transfer(env, current_admin) -> Result<(), PrediFiError>` — Cancel a pending nomination; the current admin stays in office.
- `transfer_admin(env, admin_caller, new_admin) -> Result<(), PrediFiError>` — Legacy alias for `propose_new_admin`; no longer transfers in one step.

### Types

- `Role` — Enumeration of available roles (`Admin`, `Operator`, `Moderator`, `Oracle`, `User`).
- `PoolStatus` — Enumeration of pool lifecycle states (`Active`, `Resolved`, `Closed`, `Disputed`).
- `PoolCategory` — Enumeration of pool categories (`Sports`, `Politics`, `Finance`, `Entertainment`, `Other`).
- `DataKey` — Enumeration of storage keys used by the contract.

### Events

- `AdminInitEvent` — Emitted when the contract is initialised.
- `RoleAssignedEvent` — Emitted when a role is assigned.
- `RoleRevokedEvent` — Emitted when a role is revoked.
- `RoleTransferredEvent` — Emitted when a role is transferred between users.
- `AdminTransferredEvent` — Emitted when admin rights effectively transfer (on acceptance).
- `AdminTransferProposedEvent` — Emitted when a new admin is nominated.
- `AdminTransferReplacedEvent` — Emitted when a nomination replaces an earlier one.
- `AdminTransferCancelledEvent` — Emitted when the current admin cancels a pending nomination.
- `AdminTransferAcceptedEvent` — Emitted when the nominee accepts (alongside `AdminTransferredEvent`).
- `AllRolesRevokedEvent` — Emitted when all roles are revoked from a user.

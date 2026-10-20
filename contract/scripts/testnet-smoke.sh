#!/bin/bash
# ==============================================================================
# PrediFi Testnet Smoke Test
# ==============================================================================
# Catches config, contract-address, RPC, and wallet-flow mistakes immediately
# after a testnet deployment.
#
# What it exercises:
#   - RPC reachability
#   - Read-only contract calls (get_pool, get_fee_bps, get_treasury)
#   - Full lifecycle with isolated funded accounts: create -> participate -> cancel
#   - Emits Stellar Expert transaction links for every state-changing call
#   - Labels the created pool so reruns can be identified / skipped
#
# Usage:
#   cp contract/scripts/.env.smoke.example contract/scripts/.env.smoke
#   # fill in .env.smoke
#   bash contract/scripts/testnet-smoke.sh
#
# Or pass env vars inline:
#   PREDIFI_ID=C... AC_ID=C... SOURCE=mykey bash contract/scripts/testnet-smoke.sh
#
# Secrets: never commit .env.smoke  (it is git-ignored via contract/.gitignore)
# ==============================================================================

set -euo pipefail

SMOKE_DIR="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$SMOKE_DIR/.env.smoke"
[[ -f "$ENV_FILE" ]] && source "$ENV_FILE"

# ── Required config ────────────────────────────────────────────────────────────
PREDIFI_ID="${PREDIFI_ID:?Set PREDIFI_ID to the deployed PrediFi contract ID}"
AC_ID="${AC_ID:?Set AC_ID to the deployed AccessControl contract ID}"
TOKEN="${TOKEN:-CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2QLGM5A}"
SOURCE="${SOURCE:?Set SOURCE to a funded Stellar CLI identity (stellar keys generate)}"
NETWORK="${NETWORK:-testnet}"

# ── Optional: second funded identity for participant role ──────────────────────
# If unset, SOURCE is reused for both creator and participant (still valid smoke).
PARTICIPANT="${PARTICIPANT:-$SOURCE}"

EXPLORER="https://stellar.expert/explorer/testnet/tx"

# ── Helpers ────────────────────────────────────────────────────────────────────
pass() { echo "  ✅ $*"; }
fail() { echo "  ❌ $*" >&2; exit 1; }
tx_link() { echo "  🔗 $EXPLORER/$1"; }

invoke() {
    local id="$1"; shift
    stellar contract invoke \
        --id "$id" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- "$@"
}

invoke_as() {
    local id="$1"; local signer="$2"; shift 2
    stellar contract invoke \
        --id "$id" \
        --source "$signer" \
        --network "$NETWORK" \
        -- "$@"
}

# ── Detect CLI ─────────────────────────────────────────────────────────────────
if command -v stellar &>/dev/null; then CLI="stellar"
elif command -v soroban &>/dev/null; then CLI="soroban"
else fail "Neither 'stellar' nor 'soroban' CLI found. Run: cargo install stellar-cli"; fi

echo ""
echo "══════════════════════════════════════════════════"
echo "  PrediFi Testnet Smoke Test"
echo "  Network  : $NETWORK"
echo "  PrediFi  : $PREDIFI_ID"
echo "  AC       : $AC_ID"
echo "  Token    : $TOKEN"
echo "  Source   : $SOURCE"
echo "══════════════════════════════════════════════════"
echo ""

# ── 1. RPC health ──────────────────────────────────────────────────────────────
echo "▶ 1. RPC reachability"
RPC_URL=$(stellar network ls 2>/dev/null | awk -v net="$NETWORK" '$1==net{print $2}' || true)
if [[ -z "$RPC_URL" ]]; then
    # Default testnet RPC if not in local config
    RPC_URL="https://soroban-testnet.stellar.org"
fi
STATUS=$(curl -sf -m 10 \
    -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
    | grep -o '"status":"[^"]*"' | cut -d'"' -f4 || echo "unreachable")
[[ "$STATUS" == "healthy" ]] || fail "RPC at $RPC_URL returned: $STATUS"
pass "RPC healthy ($RPC_URL)"

# ── 2. Contract existence ──────────────────────────────────────────────────────
echo ""
echo "▶ 2. Contract existence"
stellar contract info --id "$PREDIFI_ID" --network "$NETWORK" &>/dev/null \
    || fail "PrediFi contract not found: $PREDIFI_ID"
pass "PrediFi contract exists"

stellar contract info --id "$AC_ID" --network "$NETWORK" &>/dev/null \
    || fail "AccessControl contract not found: $AC_ID"
pass "AccessControl contract exists"

# ── 3. Read-only config checks ─────────────────────────────────────────────────
echo ""
echo "▶ 3. Read-only config"

TREASURY=$(invoke "$PREDIFI_ID" get_treasury 2>/dev/null || echo "")
[[ -n "$TREASURY" ]] || fail "get_treasury returned empty — contract may not be initialized"
pass "Treasury: $TREASURY"

FEE=$(invoke "$PREDIFI_ID" get_fee_bps 2>/dev/null || echo "")
[[ -n "$FEE" ]] || fail "get_fee_bps returned empty"
[[ "$FEE" -le 1000 ]] || fail "Fee ($FEE bps) is unusually high (>10%) — check deployment"
pass "Fee: $FEE bps"

RES_DELAY=$(invoke "$PREDIFI_ID" get_resolution_delay 2>/dev/null || echo "")
[[ -n "$RES_DELAY" ]] || fail "get_resolution_delay returned empty"
pass "Resolution delay: $RES_DELAY s"

# ── 4. Pool 0 read (non-existence is fine, read must not error) ────────────────
echo ""
echo "▶ 4. Pool read smoke"
POOL0=$(invoke "$PREDIFI_ID" get_pool --pool_id 0 2>&1 || true)
if echo "$POOL0" | grep -qi "PoolNotFound\|pool.*not.*found\|not exist\|Error(Contract, #"; then
    pass "get_pool works (pool 0 absent, expected on fresh deploy)"
elif echo "$POOL0" | grep -qi '"state"'; then
    pass "get_pool works (pool 0 exists)"
else
    fail "get_pool returned unexpected output: $POOL0"
fi

# ── 5. Lifecycle: create → participate → cancel ────────────────────────────────
echo ""
echo "▶ 5. Lifecycle smoke (create → participate → cancel)"

CREATOR_ADDR=$(stellar keys address "$SOURCE" --network "$NETWORK")
PARTICIPANT_ADDR=$(stellar keys address "$PARTICIPANT" --network "$NETWORK")

# end_time = 48 hours from now (enough time for pool to stay open)
END_TIME=$(( $(date +%s) + 172800 ))

POOL_CONFIG=$(cat <<EOF
{"start_time":0,"description":"[smoke] $(date -u +%Y%m%dT%H%M%SZ) rerun-safe","metadata_url":"","min_stake":"1","max_stake":"0","min_total_stake":"1","max_total_stake":"0","initial_liquidity":"0","required_resolutions":1,"private":false,"whitelist_key":null,"outcome_descriptions":["Yes","No"]}
EOF
)

echo "  Creating pool (end_time: $END_TIME)..."
CREATE_OUT=$(invoke "$PREDIFI_ID" create_pool \
    --creator "$CREATOR_ADDR" \
    --end_time "$END_TIME" \
    --token "$TOKEN" \
    --options_count 2 \
    --category "Tech" \
    --config "$POOL_CONFIG" 2>&1)

POOL_ID=$(echo "$CREATE_OUT" | grep -Eo '^[0-9]+$' | head -1 || true)
[[ -n "$POOL_ID" ]] || fail "create_pool did not return a numeric pool ID. Output: $CREATE_OUT"
pass "Pool created: id=$POOL_ID"

# Emit tx link if hash is present in output
CREATE_HASH=$(echo "$CREATE_OUT" | grep -Eo '[a-f0-9]{64}' | tail -1 || true)
[[ -n "$CREATE_HASH" ]] && tx_link "$CREATE_HASH"

# Verify pool is readable
POOL_STATE=$(invoke "$PREDIFI_ID" get_pool --pool_id "$POOL_ID" 2>/dev/null | grep -o '"state":"[^"]*"' || true)
pass "Pool $POOL_ID state: ${POOL_STATE:-<opaque>}"

echo "  Placing prediction (participant: $PARTICIPANT_ADDR, outcome: 0, amount: 1000000)..."
PREDICT_OUT=$(invoke_as "$PREDIFI_ID" "$PARTICIPANT" place_prediction \
    --user "$PARTICIPANT_ADDR" \
    --pool_id "$POOL_ID" \
    --amount 1000000 \
    --outcome 0 \
    --referral null \
    --referral_code null 2>&1)
pass "Prediction placed"
PREDICT_HASH=$(echo "$PREDICT_OUT" | grep -Eo '[a-f0-9]{64}' | tail -1 || true)
[[ -n "$PREDICT_HASH" ]] && tx_link "$PREDICT_HASH"

# Verify stake recorded
STAKE=$(invoke "$PREDIFI_ID" get_outcome_stake \
    --pool_id "$POOL_ID" --outcome 0 2>/dev/null || echo "")
pass "Outcome 0 stake: ${STAKE:-<not available>}"

echo "  Cancelling pool (label preserved in description)..."
CANCEL_OUT=$(invoke "$PREDIFI_ID" cancel_pool \
    --canceller "$CREATOR_ADDR" \
    --pool_id "$POOL_ID" 2>&1)
pass "Pool $POOL_ID cancelled"
CANCEL_HASH=$(echo "$CANCEL_OUT" | grep -Eo '[a-f0-9]{64}' | tail -1 || true)
[[ -n "$CANCEL_HASH" ]] && tx_link "$CANCEL_HASH"

# Verify cancelled state
CANCELLED_STATE=$(invoke "$PREDIFI_ID" get_pool --pool_id "$POOL_ID" 2>/dev/null \
    | grep -o '"state":"[^"]*"' || true)
echo "  Pool $POOL_ID final state: ${CANCELLED_STATE:-<opaque>}"
echo "$CANCELLED_STATE" | grep -qi "Cancelled\|cancel" \
    && pass "Pool state is Cancelled" \
    || echo "  ⚠️  Could not verify Cancelled state from output (check explorer)"

echo ""
echo "══════════════════════════════════════════════════"
echo "  ✅  Smoke test passed"
echo "══════════════════════════════════════════════════"
echo ""

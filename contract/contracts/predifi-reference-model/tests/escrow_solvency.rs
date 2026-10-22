//! # Escrow solvency invariant across all terminal paths (issue #28)
//!
//! A test-only accounting model ([`EscrowLedger`]) tracks the tokens a pool's
//! escrow actually holds and, after every state transition of [`RefModel`],
//! recomputes every outstanding obligation independently of the model:
//!
//! | Pool state           | Obligations                                                       |
//! |----------------------|-------------------------------------------------------------------|
//! | Active / Disputed    | every deposit (stakes + creator liquidity) — all are refundable    |
//! | Canceled             | unpaid stake refunds + creator liquidity                           |
//! | Resolved             | unpaid winner entitlements + treasury share (protocol fee and the  |
//! |                      | rounding / no-winner remainder) until the treasury is swept        |
//!
//! **Conservation:** `balance == obligations + surplus` after every operation.
//! The run fails on:
//! - **insolvency** — `balance < obligations + surplus`;
//! - **unallocated balance** — `balance > obligations + surplus`;
//! - **double payment** — a second payout to the same user, or a payout
//!   different from the user's independently computed entitlement.
//!
//! ## Unexpected token balance discrepancies
//!
//! - *Inbound* (tokens sent directly to the escrow, e.g. a donation): booked as
//!   `surplus`. Surplus is never counted towards claims, so it cannot mask an
//!   insolvency; it is only recoverable through an explicit admin sweep.
//! - *Outbound* (balance drops without a recorded payout, e.g. a clawback):
//!   never absorbed — the ledger reports [`Violation::Insolvent`] and
//!   settlement must halt until the shortfall is investigated.

use std::collections::{HashMap, HashSet};

use predifi_reference_model::{
    math::MAX_BPS,
    model::{FeeSchedule, MarketState, RefModel},
};
use proptest::prelude::*;

const OPTIONS: u32 = 3;

#[derive(Debug, PartialEq, Eq)]
enum Violation {
    Insolvent {
        balance: i128,
        required: i128,
    },
    Unallocated {
        balance: i128,
        required: i128,
    },
    DoublePayment {
        user: String,
    },
    WrongPayout {
        user: String,
        paid: i128,
        entitled: i128,
    },
}

/// Test-only accounting model of one pool's escrow.
#[derive(Default)]
struct EscrowLedger {
    /// Tokens actually held by the escrow.
    balance: i128,
    /// Inbound transfers not owed to anyone.
    surplus: i128,
    creator_liquidity: i128,
    /// user → (amount, outcome), from successful stakes only.
    stakes: HashMap<String, (i128, u32)>,
    paid: HashSet<String>,
    treasury_swept: bool,
}

impl EscrowLedger {
    fn new(initial_liquidity: i128) -> Self {
        Self {
            balance: initial_liquidity,
            creator_liquidity: initial_liquidity,
            ..Self::default()
        }
    }

    fn on_stake(&mut self, user: &str, amount: i128, outcome: u32) {
        self.balance += amount;
        self.stakes
            .entry(user.to_string())
            .and_modify(|(a, _)| *a += amount)
            .or_insert((amount, outcome));
    }

    fn total_deposits(&self) -> i128 {
        self.creator_liquidity + self.stakes.values().map(|(a, _)| a).sum::<i128>()
    }

    /// `(protocol_fee, payout_pool, winning_stake)` for a resolved pool,
    /// computed directly from the ledger's own records.
    fn settlement(&self, outcome: u32, fee_bps: u32) -> (i128, i128, i128) {
        let total = self.total_deposits();
        let fee = total * fee_bps as i128 / MAX_BPS;
        let winning: i128 = self
            .stakes
            .values()
            .filter(|(_, o)| *o == outcome)
            .map(|(a, _)| a)
            .sum();
        (fee, total - fee, winning)
    }

    /// What `user` is owed on the given terminal state.
    fn entitlement(&self, user: &str, state: MarketState, outcome: u32, fee_bps: u32) -> i128 {
        let Some(&(amount, chosen)) = self.stakes.get(user) else {
            return 0;
        };
        match state {
            MarketState::Canceled => amount,
            MarketState::Resolved if chosen == outcome => {
                let (_, pool, winning) = self.settlement(outcome, fee_bps);
                amount * pool / winning
            }
            _ => 0,
        }
    }

    fn treasury_share(&self, outcome: u32, fee_bps: u32) -> i128 {
        let (fee, pool, _) = self.settlement(outcome, fee_bps);
        let entitled: i128 = self
            .stakes
            .keys()
            .map(|u| self.entitlement(u, MarketState::Resolved, outcome, fee_bps))
            .sum();
        // Remainder (rounding dust, or the whole pool if nobody won) goes to the
        // treasury so that every token stays allocated.
        fee + (pool - entitled)
    }

    fn on_payout(
        &mut self,
        user: &str,
        paid: i128,
        state: MarketState,
        outcome: u32,
        fee_bps: u32,
    ) -> Result<(), Violation> {
        if !self.stakes.contains_key(user) {
            // Non-participant claims transfer nothing.
            return if paid == 0 {
                Ok(())
            } else {
                Err(Violation::WrongPayout {
                    user: user.into(),
                    paid,
                    entitled: 0,
                })
            };
        }
        if !self.paid.insert(user.to_string()) {
            return Err(Violation::DoublePayment { user: user.into() });
        }
        let entitled = self.entitlement(user, state, outcome, fee_bps);
        if paid != entitled {
            return Err(Violation::WrongPayout {
                user: user.into(),
                paid,
                entitled,
            });
        }
        self.balance -= paid;
        Ok(())
    }

    fn sweep_treasury(&mut self, outcome: u32, fee_bps: u32) -> bool {
        if self.treasury_swept {
            return false;
        }
        self.balance -= self.treasury_share(outcome, fee_bps);
        self.treasury_swept = true;
        true
    }

    /// Unexpected inbound transfer (donation / direct send).
    fn on_direct_transfer(&mut self, amount: i128) {
        self.balance += amount;
        self.surplus += amount;
    }

    /// Unexpected outbound transfer (clawback); deliberately not reconciled.
    fn on_unrecorded_outflow(&mut self, amount: i128) {
        self.balance -= amount;
    }

    fn obligations(&self, state: MarketState, outcome: u32, fee_bps: u32) -> i128 {
        let unpaid = self.stakes.keys().filter(|u| !self.paid.contains(*u));
        match state {
            MarketState::Active | MarketState::Disputed => self.total_deposits(),
            MarketState::Canceled => {
                self.creator_liquidity + unpaid.map(|u| self.stakes[u].0).sum::<i128>()
            }
            MarketState::Resolved => {
                let winners: i128 = unpaid
                    .map(|u| self.entitlement(u, state, outcome, fee_bps))
                    .sum();
                let treasury = if self.treasury_swept {
                    0
                } else {
                    self.treasury_share(outcome, fee_bps)
                };
                winners + treasury
            }
        }
    }

    fn check(&self, state: MarketState, outcome: u32, fee_bps: u32) -> Result<(), Violation> {
        let required = self.obligations(state, outcome, fee_bps) + self.surplus;
        let balance = self.balance;
        match balance.cmp(&required) {
            std::cmp::Ordering::Less => Err(Violation::Insolvent { balance, required }),
            std::cmp::Ordering::Greater => Err(Violation::Unallocated { balance, required }),
            std::cmp::Ordering::Equal => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Stake {
        user: u8,
        amount: i128,
        outcome: u32,
    },
    Resolve {
        outcome: u32,
    },
    Cancel,
    Dispute,
    Claim {
        user: u8,
    },
    ClaimRefund {
        user: u8,
    },
    SweepTreasury,
    Donate {
        amount: i128,
    },
}

/// Drives `RefModel` and the ledger in lockstep, checking conservation after
/// every operation. Returns the ledger so fixtures can inspect the end state.
fn run(fee_bps: u32, initial_liquidity: i128, ops: &[Op]) -> Result<EscrowLedger, Violation> {
    let mut model = RefModel::new();
    let pid = model
        .create_pool(
            0,
            1_000,
            1,
            0,
            0,
            0,
            initial_liquidity,
            OPTIONS,
            FeeSchedule::flat(fee_bps),
        )
        .unwrap();
    let mut ledger = EscrowLedger::new(initial_liquidity);

    for op in ops {
        let (state, outcome, bps) = {
            let p = model.pool(pid).unwrap();
            (p.state, p.outcome, p.fee_bps)
        };
        match op {
            Op::Stake {
                user,
                amount,
                outcome,
            } => {
                let user = format!("u{user}");
                if model.stake(user.as_str(), pid, *amount, *outcome).is_ok() {
                    ledger.on_stake(&user, *amount, *outcome);
                }
            }
            Op::Resolve { outcome } => {
                let _ = model.resolve(pid, *outcome, 1_500);
            }
            Op::Cancel => {
                let _ = model.cancel(pid);
            }
            Op::Dispute => {
                let _ = model.dispute(pid);
            }
            Op::Claim { user } => {
                let user = format!("u{user}");
                if let Ok(paid) = model.claim(user.as_str(), pid, 1_600) {
                    ledger.on_payout(&user, paid, state, outcome, bps)?;
                }
            }
            Op::ClaimRefund { user } => {
                let user = format!("u{user}");
                if let Ok(paid) = model.claim_refund(user.as_str(), pid) {
                    ledger.on_payout(&user, paid, state, outcome, bps)?;
                }
            }
            Op::SweepTreasury => {
                if state == MarketState::Resolved {
                    ledger.sweep_treasury(outcome, bps);
                }
            }
            Op::Donate { amount } => ledger.on_direct_transfer(*amount),
        }
        let p = model.pool(pid).unwrap();
        ledger.check(p.state, p.outcome, p.fee_bps)?;
    }
    Ok(ledger)
}

// ─────────────────────────────────────────────────────────────────────────────
// Generated lifecycle sequences
// ─────────────────────────────────────────────────────────────────────────────

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (0u8..6, 1i128..=1_000_000, 0..OPTIONS)
            .prop_map(|(user, amount, outcome)| Op::Stake { user, amount, outcome }),
        1 => (0..OPTIONS).prop_map(|outcome| Op::Resolve { outcome }),
        1 => Just(Op::Cancel),
        1 => Just(Op::Dispute),
        4 => (0u8..7).prop_map(|user| Op::Claim { user }),
        2 => (0u8..7).prop_map(|user| Op::ClaimRefund { user }),
        1 => Just(Op::SweepTreasury),
        1 => (1i128..=1_000).prop_map(|amount| Op::Donate { amount }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn prop_escrow_conserved_across_lifecycle(
        fee_bps in 0u32..=1_000,
        initial_liquidity in prop_oneof![Just(0i128), 1i128..=100_000],
        ops in prop::collection::vec(arb_op(), 1..40),
    ) {
        prop_assert_eq!(run(fee_bps, initial_liquidity, &ops).err(), None);
    }

    /// Every terminal path ends fully drained once all parties are paid.
    #[test]
    fn prop_terminal_paths_drain_to_surplus(
        fee_bps in 0u32..=1_000,
        initial_liquidity in 0i128..=100_000,
        stakes in prop::collection::vec((1i128..=1_000_000, 0..OPTIONS), 1..6),
        terminal in 0u8..3,
        winner in 0..OPTIONS,
    ) {
        let mut ops: Vec<Op> = stakes
            .iter()
            .enumerate()
            .map(|(i, &(amount, outcome))| Op::Stake { user: i as u8, amount, outcome })
            .collect();
        match terminal {
            0 => ops.push(Op::Resolve { outcome: winner }),
            1 => ops.push(Op::Cancel),
            _ => ops.extend([Op::Dispute, Op::Cancel]),
        }
        ops.extend((0..stakes.len() as u8).map(|user| Op::Claim { user }));
        ops.push(Op::SweepTreasury);

        let ledger = run(fee_bps, initial_liquidity, &ops).unwrap();
        // Only creator liquidity of a canceled pool may remain (owed to creator).
        let remaining = if terminal == 0 { 0 } else { initial_liquidity };
        prop_assert_eq!(ledger.balance, remaining);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn stake(user: u8, amount: i128, outcome: u32) -> Op {
    Op::Stake {
        user,
        amount,
        outcome,
    }
}

#[test]
fn fixture_rounding_dust_is_allocated_to_treasury() {
    // pool 10, fee 0: winners get floor(1*10/3) = 3 each, 1 unit of dust.
    let ops = [
        stake(0, 1, 0),
        stake(1, 1, 0),
        stake(2, 1, 0),
        stake(3, 7, 1),
        Op::Resolve { outcome: 0 },
        Op::Claim { user: 0 },
        Op::Claim { user: 1 },
        Op::Claim { user: 2 },
        Op::Claim { user: 3 },
    ];
    let ledger = run(0, 0, &ops).unwrap();
    assert_eq!(ledger.balance, 1, "dust remains until the treasury sweep");
    assert_eq!(ledger.treasury_share(0, 0), 1);
}

#[test]
fn fixture_no_winner_pool_is_owed_to_treasury() {
    let ops = [
        stake(0, 500, 0),
        stake(1, 500, 1),
        Op::Resolve { outcome: 2 },
        Op::Claim { user: 0 },
        Op::Claim { user: 1 },
        Op::SweepTreasury,
    ];
    assert_eq!(run(200, 0, &ops).unwrap().balance, 0);
}

#[test]
fn fixture_fee_and_liquidity_on_resolved_pool() {
    let ops = [
        stake(0, 400, 0),
        stake(1, 600, 1),
        Op::Resolve { outcome: 0 },
        Op::SweepTreasury,
        Op::Claim { user: 0 },
        Op::Claim { user: 1 },
    ];
    // total 1_100, fee 11, payout pool 1_089 all to the sole winner.
    assert_eq!(run(100, 100, &ops).unwrap().balance, 0);
}

#[test]
fn fixture_cancel_after_dispute_refunds_everyone() {
    let ops = [
        stake(0, 300, 0),
        stake(1, 700, 1),
        stake(0, 50, 0),
        Op::Dispute,
        Op::Claim { user: 0 }, // blocked while disputed
        Op::Cancel,
        Op::ClaimRefund { user: 0 },
        Op::Claim { user: 1 },
        Op::Claim { user: 0 }, // already refunded
    ];
    assert_eq!(run(500, 250, &ops).unwrap().balance, 250);
}

#[test]
fn fixture_double_claim_paths_pay_once() {
    let ops = [
        stake(0, 1_000, 0),
        Op::Cancel,
        Op::Claim { user: 0 },
        Op::ClaimRefund { user: 0 },
        Op::Claim { user: 0 },
    ];
    assert_eq!(run(0, 0, &ops).unwrap().balance, 0);
}

#[test]
fn fixture_donation_is_surplus_not_claimable() {
    let ops = [
        stake(0, 100, 0),
        Op::Donate { amount: 42 },
        Op::Resolve { outcome: 0 },
        Op::Claim { user: 0 },
        Op::SweepTreasury,
    ];
    let ledger = run(0, 0, &ops).unwrap();
    assert_eq!((ledger.balance, ledger.surplus), (42, 42));
}

#[test]
fn ledger_detects_unrecorded_outflow_as_insolvency() {
    let mut ledger = EscrowLedger::new(0);
    ledger.on_stake("u0", 100, 0);
    ledger.on_direct_transfer(10);
    ledger.on_unrecorded_outflow(5);
    // Surplus must not absorb the shortfall.
    assert_eq!(
        ledger.check(MarketState::Active, 0, 0),
        Err(Violation::Insolvent {
            balance: 105,
            required: 110
        })
    );
}

#[test]
fn ledger_detects_unallocated_balance() {
    let mut ledger = EscrowLedger::new(0);
    ledger.on_stake("u0", 100, 0);
    ledger.balance += 1; // tokens appear without being booked
    assert_eq!(
        ledger.check(MarketState::Active, 0, 0),
        Err(Violation::Unallocated {
            balance: 101,
            required: 100
        })
    );
}

#[test]
fn ledger_detects_double_and_wrong_payment() {
    let mut ledger = EscrowLedger::new(0);
    ledger.on_stake("u0", 100, 0);
    ledger.on_stake("u1", 100, 1);
    assert_eq!(
        ledger.on_payout("u0", 150, MarketState::Canceled, 0, 0),
        Err(Violation::WrongPayout {
            user: "u0".into(),
            paid: 150,
            entitled: 100
        })
    );
    ledger
        .on_payout("u1", 100, MarketState::Canceled, 0, 0)
        .unwrap();
    assert_eq!(
        ledger.on_payout("u1", 100, MarketState::Canceled, 0, 0),
        Err(Violation::DoublePayment { user: "u1".into() })
    );
}

//! Conditional-market semantics scenarios for issue #242.
//!
//! This suite is executable documentation for the proposed RFC. It does not
//! implement a contract ABI or on-chain storage; it validates graph and
//! lifecycle transitions before implementation begins.

use std::collections::{HashMap, HashSet};

const MAX_DEPTH: usize = 16;
const MAX_CHILDREN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolState {
    Active,
    Resolved(u32),
    Canceled,
    Disputed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eligibility {
    Blocked,
    Eligible,
    IneligibleCanceled,
}

#[derive(Default)]
struct ConditionalGraph {
    states: HashMap<u64, PoolState>,
    edges: HashMap<u64, (u64, u32)>,
    children: HashMap<u64, HashSet<u64>>,
    claimed: HashMap<(u64, &'static str), i128>,
}

impl ConditionalGraph {
    fn create(&mut self, pool_id: u64) {
        self.states.insert(pool_id, PoolState::Active);
    }

    fn add_dependency(
        &mut self,
        downstream: u64,
        upstream: u64,
        required_outcome: u32,
    ) -> Result<(), &'static str> {
        if downstream == upstream {
            return Err("self dependency");
        }
        if self.children.entry(upstream).or_default().len() >= MAX_CHILDREN {
            return Err("child limit");
        }
        if self.depth(upstream).unwrap_or_default() + 1 > MAX_DEPTH {
            return Err("depth limit");
        }
        if self.reaches(upstream, downstream) {
            return Err("cycle");
        }
        self.edges.insert(downstream, (upstream, required_outcome));
        self.children
            .entry(upstream)
            .or_default()
            .insert(downstream);
        Ok(())
    }

    fn reaches(&self, start: u64, target: u64) -> bool {
        let mut pending = vec![start];
        let mut visited = HashSet::new();
        while let Some(node) = pending.pop() {
            if node == target {
                return true;
            }
            if !visited.insert(node) {
                continue;
            }
            if let Some((parent, _)) = self.edges.get(&node) {
                pending.push(*parent);
            }
        }
        false
    }

    fn depth(&self, pool_id: u64) -> Option<usize> {
        fn walk(
            graph: &ConditionalGraph,
            pool_id: u64,
            visited: &mut HashSet<u64>,
        ) -> Option<usize> {
            if !visited.insert(pool_id) {
                return None;
            }
            let Some((parent, _)) = graph.edges.get(&pool_id) else {
                return Some(0);
            };
            let parent_depth = walk(graph, *parent, visited)? + 1;
            Some(parent_depth)
        }
        walk(self, pool_id, &mut HashSet::new())
    }

    fn eligibility(&self, pool_id: u64) -> Eligibility {
        let Some((upstream, required)) = self.edges.get(&pool_id).copied() else {
            return match self.states.get(&pool_id) {
                Some(PoolState::Canceled) => Eligibility::IneligibleCanceled,
                _ => Eligibility::Eligible,
            };
        };
        match self
            .states
            .get(&upstream)
            .copied()
            .unwrap_or(PoolState::Active)
        {
            PoolState::Resolved(outcome) if outcome == required => {
                match self.eligibility(upstream) {
                    Eligibility::Eligible => Eligibility::Eligible,
                    _ => Eligibility::IneligibleCanceled,
                }
            }
            PoolState::Canceled => Eligibility::IneligibleCanceled,
            PoolState::Resolved(_) => Eligibility::IneligibleCanceled,
            PoolState::Active | PoolState::Disputed => Eligibility::Blocked,
        }
    }

    fn resolve_downstream(&mut self, pool_id: u64, outcome: u32) -> Result<(), &'static str> {
        if self.eligibility(pool_id) != Eligibility::Eligible {
            return Err("upstream not resolved to required outcome");
        }
        self.states.insert(pool_id, PoolState::Resolved(outcome));
        Ok(())
    }

    fn resolve_upstream(&mut self, pool_id: u64, outcome: u32) {
        self.states.insert(pool_id, PoolState::Resolved(outcome));
    }

    fn cancel_upstream_and_descendants(&mut self, pool_id: u64) {
        self.states.insert(pool_id, PoolState::Canceled);
        let mut pending = vec![pool_id];
        let mut visited = HashSet::new();
        while let Some(parent) = pending.pop() {
            if !visited.insert(parent) {
                continue;
            }
            if let Some(children) = self.children.get(&parent) {
                for &child in children {
                    self.states.insert(child, PoolState::Canceled);
                    pending.push(child);
                }
            }
        }
    }

    fn refund(&mut self, pool_id: u64, user: &'static str, stake: i128) -> i128 {
        let key = (pool_id, user);
        if self.claimed.contains_key(&key) {
            return -1;
        }
        self.claimed.insert(key, stake);
        stake
    }

    fn claim(
        &mut self,
        pool_id: u64,
        user: &'static str,
        payout: i128,
    ) -> Result<i128, &'static str> {
        let key = (pool_id, user);
        if !matches!(self.states.get(&pool_id), Some(PoolState::Resolved(_))) {
            return Err("pool not resolved");
        }
        if self.claimed.contains_key(&key) {
            return Err("already claimed");
        }
        self.claimed.insert(key, payout);
        Ok(payout)
    }
}

#[test]
fn matching_linear_dependency_resolves_in_order() {
    let mut graph = ConditionalGraph::default();
    graph.create(10);
    graph.create(20);
    graph.create(30);
    graph.add_dependency(20, 10, 1).unwrap();
    graph.add_dependency(30, 20, 0).unwrap();

    assert_eq!(graph.eligibility(20), Eligibility::Blocked);
    assert_eq!(graph.eligibility(30), Eligibility::Blocked);
    assert_eq!(
        graph.resolve_downstream(30, 3),
        Err("upstream not resolved to required outcome")
    );

    graph.resolve_upstream(10, 1);
    assert_eq!(graph.eligibility(20), Eligibility::Eligible);
    assert_eq!(graph.eligibility(30), Eligibility::Blocked);

    graph.resolve_downstream(20, 0).unwrap();
    assert_eq!(graph.eligibility(30), Eligibility::Eligible);
    assert_eq!(graph.resolve_downstream(30, 3), Ok(()));
}

#[test]
fn upstream_mismatch_cancels_downstream_for_refund() {
    let mut graph = ConditionalGraph::default();
    graph.create(40);
    graph.create(41);
    graph.add_dependency(41, 40, 1).unwrap();

    graph.resolve_upstream(40, 2);
    assert_eq!(graph.eligibility(41), Eligibility::IneligibleCanceled);
    assert_eq!(
        graph.resolve_downstream(41, 0),
        Err("upstream not resolved to required outcome")
    );
    graph.states.insert(41, PoolState::Canceled);

    assert_eq!(graph.refund(41, "alice", 500), 500);
    assert_eq!(graph.refund(41, "alice", 500), -1);
}

#[test]
fn upstream_cancellation_propagates_across_two_levels() {
    let mut graph = ConditionalGraph::default();
    for pool_id in [50, 60, 70] {
        graph.create(pool_id);
    }
    graph.add_dependency(60, 50, 0).unwrap();
    graph.add_dependency(70, 60, 0).unwrap();

    graph.cancel_upstream_and_descendants(50);
    assert_eq!(graph.states[&50], PoolState::Canceled);
    assert_eq!(graph.states[&60], PoolState::Canceled);
    assert_eq!(graph.states[&70], PoolState::Canceled);
    assert_eq!(graph.eligibility(70), Eligibility::IneligibleCanceled);
}

#[test]
fn indirect_cycle_is_rejected_without_mutation() {
    let mut graph = ConditionalGraph::default();
    for pool_id in [80, 81, 82] {
        graph.create(pool_id);
    }
    assert_eq!(graph.add_dependency(80, 80, 0), Err("self dependency"));
    graph.add_dependency(81, 80, 0).unwrap();
    graph.add_dependency(82, 81, 0).unwrap();

    assert_eq!(graph.add_dependency(80, 82, 0), Err("cycle"));
    assert_eq!(graph.edges.len(), 2);
}

#[test]
fn direct_cycle_and_sibling_branches_are_handled_independently() {
    let mut graph = ConditionalGraph::default();
    for pool_id in [300, 301, 302, 303] {
        graph.create(pool_id);
    }
    graph.add_dependency(301, 300, 1).unwrap();
    assert_eq!(graph.add_dependency(300, 301, 1), Err("cycle"));

    graph.add_dependency(302, 300, 1).unwrap();
    graph.add_dependency(303, 300, 2).unwrap();
    graph.resolve_upstream(300, 1);

    assert_eq!(graph.eligibility(301), Eligibility::Eligible);
    assert_eq!(graph.eligibility(302), Eligibility::Eligible);
    assert_eq!(graph.eligibility(303), Eligibility::IneligibleCanceled);
    graph.states.insert(303, PoolState::Canceled);
    assert_eq!(graph.refund(303, "alice", 250), 250);
    assert_eq!(graph.refund(303, "alice", 250), -1);
}

#[test]
fn disputed_upstream_blocks_downstream_until_adjudicated() {
    let mut graph = ConditionalGraph::default();
    graph.create(90);
    graph.create(91);
    graph.add_dependency(91, 90, 0).unwrap();
    graph.states.insert(90, PoolState::Disputed);

    assert_eq!(graph.eligibility(91), Eligibility::Blocked);
    assert_eq!(
        graph.resolve_downstream(91, 0),
        Err("upstream not resolved to required outcome")
    );

    graph.states.insert(90, PoolState::Resolved(0));
    assert_eq!(graph.eligibility(91), Eligibility::Eligible);
}

#[test]
fn replayed_resolved_claim_and_canceled_refund_are_write_once() {
    let mut graph = ConditionalGraph::default();
    graph.create(92);
    graph.create(93);
    graph.add_dependency(93, 92, 0).unwrap();

    graph.resolve_upstream(92, 0);
    graph.resolve_downstream(93, 1).unwrap();
    assert_eq!(graph.claim(93, "alice", 700), Ok(700));
    assert_eq!(graph.claim(93, "alice", 700), Err("already claimed"));

    graph.create(94);
    graph.create(95);
    graph.add_dependency(95, 94, 0).unwrap();
    graph.cancel_upstream_and_descendants(94);
    assert_eq!(graph.refund(95, "bob", 300), 300);
    assert_eq!(graph.refund(95, "bob", 300), -1);
}

#[test]
fn depth_and_fanout_limits_are_enforced() {
    let mut graph = ConditionalGraph::default();
    for pool_id in 100..118 {
        graph.create(pool_id);
    }

    for pool_id in 101..117 {
        let upstream = pool_id - 1;
        graph.add_dependency(pool_id, upstream, 0).unwrap();
    }
    assert_eq!(graph.depth(116), Some(16));
    assert_eq!(graph.add_dependency(117, 116, 0), Err("depth limit"));

    let mut fanout = ConditionalGraph::default();
    fanout.create(200);
    for pool_id in 201..266 {
        fanout.create(pool_id);
        if pool_id <= 264 {
            fanout.add_dependency(pool_id, 200, 0).unwrap();
        }
    }
    assert_eq!(fanout.add_dependency(265, 200, 0), Err("child limit"));
}

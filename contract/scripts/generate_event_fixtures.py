#!/usr/bin/env python3
"""
Generate deterministic contract event JSON fixtures from contract event schema tests
and release artifact schemas for external indexers.
"""
import json
import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVENT_SCHEMA_TESTS = ROOT / "contracts" / "predifi-contract" / "tests" / "event_schema_tests.rs"
OUTPUT_PATH = ROOT / "fixtures" / "event_fixtures_v1.json"

CANONICAL_FIXTURES = {
    "schema_version": 1,
    "contract_version": 1,
    "source": "contract/contracts/predifi-contract/tests/event_schema_tests.rs",
    "events": [
        {
            "topic": "init",
            "category": "pool_lifecycle",
            "payload": {
                "access_control": "CDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
                "treasury": "GDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
                "fee_bps": 100,
                "resolution_delay": 3600,
                "min_pool_duration": 300,
                "max_predictions_per_user": 0,
            },
        },
        {
            "topic": "pool_created",
            "category": "pool_lifecycle",
            "payload": {
                "pool_id": 1,
                "creator": "GDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
                "end_time": 1767225600,
                "token": "CDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
                "options_count": 2,
                "metadata_url": "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
                "initial_liquidity": "10000000",
                "category": "Crypto",
                "required_resolutions": 2,
                "max_total_stake": "0",
                "outcome_descriptions": ["Yes", "No"],
            },
        },
        {
            "topic": "prediction_placed",
            "category": "prediction",
            "payload": {
                "pool_id": 1,
                "user": "GDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
                "option": 0,
                "amount": "5000000",
                "referrer": None,
            },
        },
        {
            "topic": "pool_resolved",
            "category": "resolution",
            "payload": {
                "pool_id": 1,
                "winning_option": 0,
                "resolved_at": 1767229200,
            },
        },
        {
            "topic": "winnings_claimed",
            "category": "payout",
            "payload": {
                "pool_id": 1,
                "user": "GDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
                "payout": "14850000",
                "fee": "150000",
            },
        },
    ],
}


def build_fixtures() -> dict:
    data = dict(CANONICAL_FIXTURES)
    if EVENT_SCHEMA_TESTS.exists():
        text = EVENT_SCHEMA_TESTS.read_text(encoding="utf-8", errors="ignore")
        discovered_topics = sorted(set(re.findall(r'symbol_short!\("([a-zA-Z0-9_]+)"\)', text)))
        if discovered_topics:
            data["verified_test_topics"] = discovered_topics
    return data


def main() -> None:
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    fixtures = build_fixtures()
    OUTPUT_PATH.write_text(json.dumps(fixtures, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote contract event fixtures to {OUTPUT_PATH}")


if __name__ == "__main__":
    main()

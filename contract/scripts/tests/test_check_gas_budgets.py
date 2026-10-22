"""Unit tests for scripts/check_gas_budgets.py.

Run: python3 -m unittest discover -s contract/scripts/tests -p 'test_check_gas_budgets.py'
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from check_gas_budgets import check, parse_measurements  # noqa: E402

METRICS = ["cpu_insns", "mem_bytes"]


def budgets(baseline=None):
    return {
        "alert_threshold_pct": 80,
        "metrics": METRICS,
        "entry_points": {
            "create_pool": {
                "ceiling": {"cpu_insns": 1000, "mem_bytes": 100},
                "baseline": baseline,
            }
        },
    }


def line(entry, cpu, mem):
    return f'test x ... GAS_BUDGET {{"entry_point":"{entry}","cpu_insns":{cpu},"mem_bytes":{mem}}}\n'


class CheckGasBudgetsTest(unittest.TestCase):
    def test_parses_lines_interleaved_with_test_output(self):
        text = "running 2 tests\n" + line("create_pool", 10, 2) + "test result: ok\n"
        self.assertEqual(
            parse_measurements(text), {"create_pool": {"cpu_insns": 10, "mem_bytes": 2}}
        )

    def test_within_budget_passes(self):
        _, failures, warnings = check(budgets(), parse_measurements(line("create_pool", 10, 2)))
        self.assertEqual((failures, warnings), ([], []))

    def test_alert_threshold_warns_without_failing(self):
        _, failures, warnings = check(budgets(), parse_measurements(line("create_pool", 800, 2)))
        self.assertEqual(failures, [])
        self.assertEqual(len(warnings), 1)

    def test_ceiling_fails(self):
        _, failures, _ = check(budgets(), parse_measurements(line("create_pool", 1000, 2)))
        self.assertEqual(failures, ["create_pool.cpu_insns: 1000 >= ceiling 1000"])

    def test_missing_entry_point_fails(self):
        _, failures, _ = check(budgets(), {})
        self.assertEqual(len(failures), 1)

    def test_regression_against_baseline_is_reported_not_blocking(self):
        rows, failures, _ = check(
            budgets({"cpu_insns": 10, "mem_bytes": 2}),
            parse_measurements(line("create_pool", 12, 2)),
        )
        self.assertEqual(failures, [])
        self.assertEqual(rows[0][4:], ("+2", 1000, "1.20%", "regressed"))
        self.assertEqual(rows[1][-1], "ok")


if __name__ == "__main__":
    unittest.main()

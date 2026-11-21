"""crap_report 的回归测试：公式、函数自身覆盖率归属、受检范围与基线 ratchet。"""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import crap_report


def function(name: str, start: int, end: int, complexity: int) -> dict:
    return {"name": name, "start": {"line": start, "column": 0}, "end": {"line": end, "column": 1},
            "complexity": complexity}


def statement(line: int) -> dict:
    return {"start": {"line": line, "column": 0}, "end": {"line": line, "column": 10}}


class FormulaTest(unittest.TestCase):
    def test_matches_android_definition(self) -> None:
        self.assertEqual(crap_report.crap(5, 1.0), 5)
        self.assertEqual(crap_report.crap(3, 0.0), 12)
        self.assertAlmostEqual(crap_report.crap(4, 0.5), 4 * 4 * 0.125 + 4)

    def test_nested_functions_do_not_lend_coverage_to_the_outer_function(self) -> None:
        functions = [function("outer", 1, 10, 4), function("inner", 3, 5, 1)]
        coverage = {
            "statementMap": {"0": statement(2), "1": statement(4), "2": statement(8)},
            "s": {"0": 1, "1": 5, "2": 0},
            "branchMap": {"0": {"loc": statement(8), "locations": [statement(8)]}},
            "b": {"0": [0]},
        }
        outer, inner = crap_report.function_coverage(functions, coverage)
        # outer 自身两行只覆盖一行，块覆盖为 0，取较低者
        self.assertEqual(outer, 0.0)
        self.assertEqual(inner, 1.0)

    def test_missing_coverage_counts_as_uncovered(self) -> None:
        self.assertEqual(crap_report.function_coverage([function("f", 1, 3, 2)], None), [0.0])


class GateTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        for name in ("src/modules/a.ts", "src/modules/b.ts"):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("export const x = 1\n", encoding="utf-8")
        self.config = {"measure_patterns": ["src/modules/**/*.ts"], "source_patterns": ["src/modules/a.ts"],
                       "unscoped_failure_baseline": 1}
        self.complexity = {
            "src/modules/a.ts": [function("covered", 1, 3, 6)],
            "src/modules/b.ts": [function("untested", 1, 3, 3)],
        }
        self.coverage = {
            str(self.root / "src/modules/a.ts"): {"statementMap": {"0": statement(2)}, "s": {"0": 3},
                                                  "branchMap": {}, "b": {}},
        }

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def run_gate(self) -> tuple[int, str]:
        (self.root / "config/quality").mkdir(parents=True, exist_ok=True)
        (self.root / "config/quality/crap-scope.json").write_text(json.dumps(self.config), encoding="utf-8")
        (self.root / "complexity.json").write_text(json.dumps(self.complexity), encoding="utf-8")
        (self.root / "coverage.json").write_text(json.dumps(self.coverage), encoding="utf-8")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = crap_report.main(["--root", str(self.root), "--coverage", str(self.root / "coverage.json"),
                                     "--complexity", str(self.root / "complexity.json"),
                                     "--output", str(self.root / "out")])
        return code, output.getvalue()

    def test_passes_when_scope_is_clean_and_baseline_matches(self) -> None:
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)
        self.assertIn("2 functions, 1 > 8; 1 scoped functions, 0 > 9", output)
        self.assertTrue((self.root / "out/above-8.md").is_file())

    def test_scoped_function_above_nine_fails(self) -> None:
        self.coverage = {}
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("FAIL src/modules/a.ts:1 covered C=6", output)

    def test_unscoped_baseline_only_moves_down(self) -> None:
        self.config["unscoped_failure_baseline"] = 0
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("1 unscoped functions exceed CRAP 9, baseline is 0", output)
        self.config["unscoped_failure_baseline"] = 2
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("lower unscoped_failure_baseline", output)

    def test_scope_patterns_must_match_measured_sources(self) -> None:
        self.config["source_patterns"] = ["src/modules/gone.ts"]
        with contextlib.redirect_stderr(io.StringIO()) as errors:
            code, _ = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn("matches no measured sources", errors.getvalue())


if __name__ == "__main__":
    unittest.main()

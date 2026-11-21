"""maintainability 度量的回归测试：未测量的项目不能得分，静态指标要统计到真实文件。"""

from __future__ import annotations

import json
import tempfile
import textwrap
import unittest
from pathlib import Path

import maintainability


class ScoringTest(unittest.TestCase):
    def test_unmeasured_items_score_zero(self) -> None:
        self.assertEqual(maintainability.flag(None), 0.0)
        self.assertEqual(maintainability.ratio(None, 70), 0.0)
        self.assertEqual(maintainability.decay(None, 10), 0.0)
        # bool 是 int 的子类，不能被当成数值指标
        self.assertEqual(maintainability.ratio(True, 1), 0.0)

    def test_decay_and_ratio_are_clamped(self) -> None:
        self.assertEqual(maintainability.decay(0, 10), 1.0)
        self.assertAlmostEqual(maintainability.decay(5, 10), 0.5)
        self.assertEqual(maintainability.decay(50, 10), 0.0)
        self.assertEqual(maintainability.ratio(140, 70), 1.0)

    def test_rubric_totals_one_hundred(self) -> None:
        result = maintainability.Result()
        maintainability.score(result)
        self.assertEqual(sum(item.max_points for item in result.scores), 100)
        self.assertEqual(sum(item.points for item in result.scores), 0)


class StaticCollectionTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(textwrap.dedent(content), encoding="utf-8")

    def test_counts_crates_tests_and_any(self) -> None:
        self.write("src-tauri/crates/neri-a/Cargo.toml", '[package]\nname = "neri-a"\n')
        self.write("src-tauri/crates/neri-a/README.md", "# a\n")
        self.write("src-tauri/crates/neri-a/src/lib.rs", "pub fn a() {}\n#[cfg(test)]\nmod tests {\n#[test]\nfn t() {}\n}\n")
        self.write("src-tauri/crates/neri-b/Cargo.toml", '[package]\nname = "neri-b"\n[dependencies]\ntauri = "2"\n')
        self.write("src-tauri/crates/neri-b/src/lib.rs", "#[tokio::test]\nasync fn t() {}\n")
        self.write("src/stores/auth.ts", "export const a = (x: any) => x as any\n// const b: any\n")
        self.write("src/stores/__tests__/auth.test.ts", "it('a', () => {})\ntest('b', () => {})\n")
        self.write("src/stores/player.ts", "export const p = 1\n")
        self.write("package.json", json.dumps({"scripts": {"test:a": "node a.mjs", "build": "vite"}}))
        (self.root / ".github" / "workflows").mkdir(parents=True)
        (self.root / ".github" / "workflows" / "ci.yml").write_text("run: pnpm typecheck\n", encoding="utf-8")

        result = maintainability.Result()
        maintainability.collect_static(self.root, result)
        self.assertEqual(result.get("rust.crates"), 2)
        self.assertEqual(result.get("rust.crates_with_readme"), 1)
        self.assertEqual(result.get("rust.tauri_free_crates"), 1)
        self.assertEqual(result.get("rust.test_functions"), 2)
        self.assertEqual(result.get("frontend.any_annotations"), 2)
        self.assertEqual(result.get("frontend.vitest_cases"), 2)
        self.assertEqual(result.get("frontend.stores"), 2)
        self.assertEqual(result.get("frontend.stores_with_unit_tests"), 1)
        self.assertEqual(result.get("frontend.regression_scripts"), 1)
        self.assertEqual(result.get("process.ci_checks"), 1)


if __name__ == "__main__":
    unittest.main()

"""module_boundaries 的回归测试：用临时夹具仓库验证违规会被拒绝，合规结构会通过。"""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import textwrap
import unittest
from pathlib import Path

import module_boundaries

CONFIG = {
    "rust": {
        "workspace": "src-tauri",
        "crates_dir": "src-tauri/crates",
        "app_crate": "app",
        "require_readme": True,
        "forbidden_external_prefixes": ["tauri"],
        "max_production_lines": 20,
        "max_rs_files_per_directory": 3,
        "crates": {
            "neri-base": {"allowed": []},
            "neri-domain": {"allowed": ["neri-base"]},
        },
    },
    "frontend": {
        "root": "src",
        "max_lines": 10,
        "max_files_per_directory": 4,
        "layers": {
            "types": ["types"],
            "modules": ["types", "modules"],
            "stores": ["types", "modules", "stores", "i18n"],
            "i18n": ["i18n"],
            "views": ["types", "modules", "stores", "i18n"],
            "(root)": "*",
        },
        "ipc_layers": ["types", "modules", "stores"],
        "legacy_oversized": {},
        "ui_invoke_baseline": {},
    },
}


def crate_manifest(name: str, deps: dict[str, str] | None = None) -> str:
    lines = [f'[package]\nname = "{name}"\nversion = "0.1.0"\n', "[dependencies]"]
    for dep, spec in (deps or {}).items():
        lines.append(f"{dep} = {spec}")
    return "\n".join(lines) + "\n"


class FixtureRepo:
    def __init__(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.config = json.loads(json.dumps(CONFIG))
        self.write("src-tauri/Cargo.toml", crate_manifest("app", {
            "neri-base": '{ path = "crates/neri-base" }',
            "neri-domain": '{ path = "crates/neri-domain" }',
            "tauri": '"2"',
        }))
        self.write("src-tauri/src/main.rs", "fn main() {}\n")
        self.add_crate("neri-base")
        self.add_crate("neri-domain", {"neri-base": '{ path = "../neri-base" }'})
        self.write("src/types/track.ts", "export interface Track { id: string }\n")
        self.write("src/modules/queue.ts", "import type { Track } from '@/types/track'\nexport const q: Track[] = []\n")
        self.write("src/stores/player.ts", "import { q } from '@/modules/queue'\nimport { invoke } from '@tauri-apps/api/core'\nexport const s = () => invoke('play', { q })\n")
        self.write("src/i18n/index.ts", "export default {}\n")
        self.write("src/views/Home.vue", "<script setup lang=\"ts\">\nimport i18n from '@/i18n'\nimport { s } from '@/stores/player'\n</script>\n")

    def add_crate(self, name: str, deps: dict[str, str] | None = None) -> None:
        self.write(f"src-tauri/crates/{name}/Cargo.toml", crate_manifest(name, deps))
        self.write(f"src-tauri/crates/{name}/README.md", f"# {name}\n")
        self.write(f"src-tauri/crates/{name}/src/lib.rs", "pub fn ok() {}\n")

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(textwrap.dedent(content), encoding="utf-8")

    def run(self) -> tuple[int, str]:
        self.write("config/quality/module-boundaries.json", json.dumps(self.config))
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = module_boundaries.run(self.root)
        return code, output.getvalue()

    def close(self) -> None:
        self._tmp.cleanup()


class ModuleBoundariesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = FixtureRepo()

    def tearDown(self) -> None:
        self.repo.close()

    def assert_violation(self, fragment: str) -> None:
        code, output = self.repo.run()
        self.assertEqual(code, 1, output)
        self.assertIn(fragment, output)

    def test_compliant_fixture_passes(self) -> None:
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.assertIn("Rust module boundaries: OK", output)
        self.assertIn("Frontend module boundaries: OK", output)

    def test_unregistered_crate_is_rejected(self) -> None:
        self.repo.add_crate("neri-rogue")
        self.assert_violation("crate neri-rogue exists on disk but is not registered")

    def test_disallowed_project_dependency_is_rejected(self) -> None:
        self.repo.write("src-tauri/crates/neri-base/Cargo.toml", crate_manifest("neri-base", {"neri-domain": '{ path = "../neri-domain" }'}))
        self.assert_violation("neri-base must not depend on neri-domain")

    def test_dependency_cycle_is_reported(self) -> None:
        self.repo.config["rust"]["crates"]["neri-base"]["allowed"] = ["neri-domain"]
        self.repo.write("src-tauri/crates/neri-base/Cargo.toml", crate_manifest("neri-base", {"neri-domain": '{ path = "../neri-domain" }'}))
        self.assert_violation("dependency cycle")

    def test_dev_and_target_dependencies_are_checked(self) -> None:
        self.repo.write("src-tauri/crates/neri-base/Cargo.toml",
                        crate_manifest("neri-base") + '\n[target.\'cfg(unix)\'.dev-dependencies]\nneri-domain = { path = "../neri-domain" }\n')
        self.assert_violation("neri-base must not depend on neri-domain")

    def test_domain_crate_cannot_depend_on_tauri(self) -> None:
        self.repo.write("src-tauri/crates/neri-domain/Cargo.toml", crate_manifest("neri-domain", {
            "neri-base": '{ path = "../neri-base" }', "tauri-plugin-store": '"2"'}))
        self.assert_violation("domain crate neri-domain must not depend on tauri-plugin-store")

    def test_domain_source_cannot_reference_tauri_apis(self) -> None:
        self.repo.write("src-tauri/crates/neri-domain/src/lib.rs", "pub fn f() { let _ = tauri::async_runtime::handle(); }\n")
        self.assert_violation("references tauri APIs")

    def test_missing_readme_is_reported(self) -> None:
        (self.repo.root / "src-tauri/crates/neri-base/README.md").unlink()
        self.assert_violation("neri-base is missing README.md")

    def test_package_name_must_match_directory(self) -> None:
        self.repo.write("src-tauri/crates/neri-base/Cargo.toml", crate_manifest("neri-renamed"))
        self.assert_violation("declares package name 'neri-renamed'")

    def test_oversized_production_file_is_rejected_but_inline_tests_are_excluded(self) -> None:
        body = "pub fn f() {}\n" * 15
        tests = "#[cfg(test)]\nmod tests {\n" + "    #[test]\n    fn t() {}\n" * 10 + "}\n"
        self.repo.write("src-tauri/crates/neri-base/src/lib.rs", body + tests)
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.repo.write("src-tauri/crates/neri-base/src/lib.rs", "pub fn f() {}\n" * 25)
        self.assert_violation("has 25 production lines (limit 20)")

    def test_separate_tests_file_is_not_counted(self) -> None:
        self.repo.write("src-tauri/crates/neri-base/src/tests.rs", "#[test]\nfn t() {}\n" * 30)
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_crowded_directory_is_rejected(self) -> None:
        for index in range(3):
            self.repo.write(f"src-tauri/crates/neri-base/src/m{index}.rs", "pub fn f() {}\n")
        self.assert_violation("has 4 .rs files (limit 3)")

    def test_frontend_layer_violation_is_rejected(self) -> None:
        self.repo.write("src/modules/queue.ts", "import { s } from '@/stores/player'\nexport const q = [s]\n")
        self.assert_violation("src/modules/queue.ts (modules) must not import @/stores/player (stores)")

    def test_relative_import_across_layers_is_resolved(self) -> None:
        self.repo.write("src/modules/queue.ts", "import { s } from '../stores/player'\nexport const q = [s]\n")
        self.assert_violation("must not import ../stores/player (stores)")

    def test_commented_imports_are_ignored(self) -> None:
        self.repo.write("src/modules/queue.ts", "// import { s } from '@/stores/player'\n/* import x from '@/stores/player' */\nexport const q = []\n")
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_directory_entry_import_maps_to_layer(self) -> None:
        self.repo.write("src/modules/queue.ts", "import i18n from '@/i18n'\nexport const q = [i18n]\n")
        self.assert_violation("must not import @/i18n (i18n)")

    def test_unregistered_frontend_layer_is_rejected(self) -> None:
        self.repo.write("src/widgets/Card.vue", "<template><div /></template>\n")
        self.assert_violation("is in unregistered layer 'widgets'")

    def test_ui_layer_invoke_requires_baseline(self) -> None:
        self.repo.write("src/views/Home.vue", "<script setup lang=\"ts\">\nimport { invoke } from '@tauri-apps/api/core'\ninvoke<string>('x')\n</script>\n")
        self.assert_violation("has 1 direct invoke() calls (baseline 0)")
        self.repo.config["frontend"]["ui_invoke_baseline"] = {"src/views/Home.vue": 1}
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_invoke_baseline_must_shrink_with_code(self) -> None:
        self.repo.config["frontend"]["ui_invoke_baseline"] = {"src/views/Home.vue": 2}
        self.assert_violation("lower ui_invoke_baseline for src/views/Home.vue from 2 to 0")

    def test_oversized_frontend_file_and_legacy_ratchet(self) -> None:
        self.repo.write("src/stores/player.ts", "export const a = 1\n" * 12)
        self.assert_violation("has 12 lines (limit 10)")
        self.repo.config["frontend"]["legacy_oversized"] = {"src/stores/player.ts": 11}
        self.assert_violation("legacy oversized src/stores/player.ts grew to 12 lines (ceiling 11)")
        self.repo.config["frontend"]["legacy_oversized"] = {"src/stores/player.ts": 12}
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.repo.config["frontend"]["legacy_oversized"] = {"src/stores/player.ts": 13}
        self.assert_violation("lower legacy_oversized ceiling for src/stores/player.ts from 13 to 12")
        self.repo.config["frontend"]["legacy_oversized"] = {"src/stores/player.ts": 12}
        self.repo.write("src/stores/player.ts", "export const a = 1\n")
        self.assert_violation("remove it from legacy_oversized")

    def test_crowded_directory_ratchet(self) -> None:
        for index in range(4):
            self.repo.write(f"src/modules/extra{index}.ts", "export const x = 1\n")
        self.assert_violation("src/modules has 5 source files (limit 4)")
        self.repo.config["frontend"]["legacy_crowded_directories"] = {"src/modules": 5}
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.repo.config["frontend"]["legacy_crowded_directories"] = {"src/modules": 6}
        self.assert_violation("lower legacy_crowded_directories ceiling for src/modules from 6 to 5")
        (self.repo.root / "src/modules/extra0.ts").unlink()
        self.repo.config["frontend"]["legacy_crowded_directories"] = {"src/modules": 5}
        self.assert_violation("remove it from legacy_crowded_directories")

    def test_tests_and_declarations_are_not_frontend_sources(self) -> None:
        self.repo.write("src/modules/__tests__/queue.test.ts", "import { s } from '@/stores/player'\n" * 20)
        self.repo.write("src/modules/shims.d.ts", "declare module 'x'\n" * 20)
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)


if __name__ == "__main__":
    unittest.main()

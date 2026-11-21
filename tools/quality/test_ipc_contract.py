"""ipc_contract 的回归测试：命令定义、注册与前端调用不一致时必须失败。"""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import textwrap
import unittest
from pathlib import Path

import ipc_contract

COMMANDS = """
#[tauri::command]
pub async fn play(state: State<'_, AppState>) -> AppResult<()> { Ok(()) }

/// 带文档注释与额外属性的命令
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub fn stop() {}
"""

MAIN = """
fn main() {
    tauri::Builder::default().invoke_handler(tauri::generate_handler![
        player_cmd::play,
        // 注释掉的条目不算注册
        // player_cmd::legacy,
        player_cmd::stop,
    ]);
}
"""


class FixtureRepo:
    def __init__(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.config = {
            "commands_dir": "src-tauri/src/commands",
            "handler_file": "src-tauri/src/main.rs",
            "frontend_root": "src",
            "dynamic_invocations": {},
            "unreferenced_baseline": [],
        }
        self.write("src-tauri/src/commands/player_cmd.rs", COMMANDS)
        self.write("src-tauri/src/main.rs", MAIN)
        self.write("src/stores/player.ts", "import { invoke } from '@tauri-apps/api/core'\n"
                   "export const play = () => invoke<void>('play')\n"
                   "export const stop = () => invoke(\"stop\", {})\n")

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(textwrap.dedent(content), encoding="utf-8")

    def run(self) -> tuple[int, str]:
        self.write("config/quality/ipc-contract.json", json.dumps(self.config))
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = ipc_contract.run(self.root)
        return code, output.getvalue()

    def close(self) -> None:
        self._tmp.cleanup()


class IpcContractTest(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = FixtureRepo()

    def tearDown(self) -> None:
        self.repo.close()

    def assert_violation(self, fragment: str) -> None:
        code, output = self.repo.run()
        self.assertEqual(code, 1, output)
        self.assertIn(fragment, output)

    def test_consistent_fixture_passes(self) -> None:
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.assertIn("2 defined, 2 registered, 2 invoked by literal", output)

    def test_defined_but_unregistered_command_fails(self) -> None:
        self.repo.write("src-tauri/src/commands/player_cmd.rs", COMMANDS + "\n#[tauri::command]\npub fn seek() {}\n")
        self.assert_violation("player_cmd::seek is defined but not registered")

    def test_registered_without_definition_fails(self) -> None:
        self.repo.write("src-tauri/src/main.rs", MAIN.replace("player_cmd::stop,", "player_cmd::stop,\n        player_cmd::ghost,"))
        self.assert_violation("generate_handler! entry player_cmd::ghost has no matching")

    def test_unknown_frontend_command_fails(self) -> None:
        self.repo.write("src/views/Home.vue", "<script setup>\ninvoke<Map<string, number>>('plya')\n</script>\n")
        self.assert_violation("invoke('plya')")

    def test_commented_invocations_are_ignored(self) -> None:
        self.repo.write("src/views/Home.vue", "<script setup>\n// invoke('plya')\n/* invoke('ghost') */\n</script>\n")
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_unreferenced_command_requires_baseline(self) -> None:
        self.repo.write("src/stores/player.ts", "export const play = () => invoke('play')\n")
        self.assert_violation("command stop is registered but never invoked")
        self.repo.config["unreferenced_baseline"] = ["stop"]
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_stale_baseline_entry_fails(self) -> None:
        self.repo.config["unreferenced_baseline"] = ["stop"]
        self.assert_violation("unreferenced_baseline entry stop is now referenced")

    def test_dynamic_invoke_requires_declared_literal_candidates(self) -> None:
        self.repo.write("src/stores/player.ts",
                        "const run = (command: string) => invoke(command)\nrun('play')\nrun('stop')\n")
        self.assert_violation("non-literal command")
        self.repo.config["dynamic_invocations"] = {"src/stores/player.ts": ["play", "stop"]}
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.repo.config["dynamic_invocations"] = {"src/stores/player.ts": ["play", "stop", "seek"]}
        self.assert_violation("dynamic candidate seek for src/stores/player.ts is not a registered command")

    def test_dynamic_declaration_without_dynamic_call_is_stale(self) -> None:
        self.repo.config["dynamic_invocations"] = {"src/stores/player.ts": ["play"]}
        self.assert_violation("no longer has a non-literal invoke()")

    def test_duplicate_registration_fails(self) -> None:
        self.repo.write("src-tauri/src/main.rs", MAIN.replace("player_cmd::stop,", "player_cmd::stop,\n        player_cmd::stop,"))
        self.assert_violation("registered more than once")

    def test_test_files_are_not_frontend_callers(self) -> None:
        self.repo.write("src/stores/__tests__/player.test.ts", "invoke('missing_command')\n")
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_nested_command_modules_match_reexported_paths(self) -> None:
        self.repo.write("src-tauri/src/commands/player_cmd.rs", "")
        self.repo.write("src-tauri/src/commands/player_cmd/mod.rs", "mod control;\npub use control::*;\n")
        self.repo.write("src-tauri/src/commands/player_cmd/control.rs", COMMANDS)
        self.repo.write("src-tauri/src/commands/player_cmd/tests.rs", "#[tauri::command]\npub fn helper() {}\n")
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)
        self.repo.write("src-tauri/src/commands/discover/search.rs", "#[tauri::command]\npub fn play() {}\n")
        self.repo.write("src-tauri/src/main.rs", MAIN.replace("player_cmd::stop,", "player_cmd::stop,\n        discover::play,"))
        self.repo.write("src/stores/player.ts", "invoke('play')\ninvoke('stop')\n")
        self.assert_violation("command name play is ambiguous across modules")

    def test_nested_generic_invocations_are_literal(self) -> None:
        self.repo.write("src/stores/player.ts",
                        "export const play = () => invoke<Array<Record<string, unknown> | null>>('play')\n"
                        "export const stop = () => invoke('stop')\n")
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_wrapper_calls_count_as_references(self) -> None:
        self.repo.write("src/modules/userData.ts",
                        "export function persistUserData<T>(command: string, args: object): Promise<T> {\n"
                        "  return invoke<T>(command, args)\n}\n")
        self.repo.write("src/stores/player.ts",
                        "function persist(command: string) { void persistUserData(command, {}) }\n"
                        "persist('play')\nawait persistUserData<void>('stop', {})\n")
        self.repo.config["invoke_wrappers"] = [
            {"name": "persistUserData", "file": "src/modules/userData.ts", "scope": "global"},
            {"name": "persist", "file": "src/stores/player.ts", "scope": "file"},
        ]
        code, output = self.repo.run()
        self.assertEqual(code, 0, output)

    def test_wrapper_misuse_fails(self) -> None:
        self.repo.write("src/modules/userData.ts",
                        "export function persistUserData(command: string) { return invoke(command) }\n")
        self.repo.write("src/views/Home.vue", "<script setup>\nconst name = 'play'\npersistUserData(name)\n</script>\n")
        self.repo.config["invoke_wrappers"] = [
            {"name": "persistUserData", "file": "src/modules/userData.ts", "scope": "global"},
        ]
        self.assert_violation("src/views/Home.vue calls persistUserData(name) with a non-literal command")
        self.repo.config["invoke_wrappers"] = [
            {"name": "persistUserData", "file": "src/modules/missing.ts", "scope": "global"},
        ]
        self.assert_violation("points to missing file src/modules/missing.ts")
        self.repo.write("src/stores/player.ts", "export const persistUserData = () => invoke('play')\ninvoke('stop')\n")
        self.repo.config["invoke_wrappers"] = [
            {"name": "persistUserData", "file": "src/stores/player.ts", "scope": "global"},
        ]
        self.assert_violation("is not defined as a function in src/stores/player.ts")


if __name__ == "__main__":
    unittest.main()

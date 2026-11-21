"""IPC 契约门禁：Rust `#[tauri::command]` 定义、`generate_handler!` 注册表与前端 `invoke()` 调用三方一致。

- 每个命令必须注册，注册项必须指向真实命令；命令可以放在 commands 的子目录里
- 前端调用的命令名必须已注册（拼写错误或残留调用会在运行时才失败）
- 变量形式的调用须在配置中声明候选命令，且候选名必须以字面量出现在该文件
- 转发命令名的封装函数（如 persistUserData）在配置中登记后，按其调用处的字面量计入引用；
  封装函数所在文件内把变量命令名继续转发给 invoke 或其它封装是允许的
- 未被前端引用的命令只允许出现在基线中，且基线只减不增

用法：python3 -B tools/quality/ipc_contract.py [--root <repo>]
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

from quality_common import GENERIC_ARGS, INVOKE_CALL, REPO_ROOT, STRING_LITERAL, Report, load_json, rel, strip_comments

CONFIG = Path("config/quality/ipc-contract.json")
COMMAND_FN = re.compile(
    r"#\[tauri::command(?:\([^)]*\))?\]\s*(?:(?:#\[[^\]]*\]|///[^\n]*|//[^\n]*)\s*)*"
    r"pub\s+(?:async\s+)?fn\s+(\w+)"
)
HANDLER = re.compile(r"generate_handler!\s*\[(.*?)\]", re.S)


@dataclass(frozen=True)
class Definition:
    name: str
    display: str
    candidates: frozenset[str]
    path: str


def defined_commands(commands_dir: Path) -> list[Definition]:
    """递归收集命令定义。

    `commands/a/b.rs` 里的命令可以经 `a::b::f`、`b::f` 或父模块重导出的 `a::f` 注册，
    这些写法都视为指向同一个定义；`mod.rs` 不占模块名，`tests.rs` 不参与。
    """
    result: list[Definition] = []
    for path in sorted(commands_dir.rglob("*.rs")):
        if path.name == "tests.rs":
            continue
        parts = [*path.relative_to(commands_dir).parent.parts]
        if path.stem != "mod":
            parts.append(path.stem)
        if not parts:
            continue
        for match in COMMAND_FN.finditer(path.read_text(encoding="utf-8")):
            name = match.group(1)
            modules = {"::".join(parts[index:]) for index in range(len(parts))} | {parts[0]}
            result.append(Definition(name, "::".join([*parts, name]),
                                     frozenset(f"{module}::{name}" for module in modules), str(path)))
    return result


def registered_commands(main_rs: Path) -> list[str]:
    match = HANDLER.search(main_rs.read_text(encoding="utf-8"))
    if not match:
        return []
    body = re.sub(r"//[^\n]*", "", match.group(1))
    return [entry.strip() for entry in body.split(",") if entry.strip()]


def frontend_invocations(src_root: Path) -> tuple[dict[str, set[str]], dict[str, list[str]], dict[str, str]]:
    literal: dict[str, set[str]] = {}
    dynamic: dict[str, list[str]] = {}
    sources: dict[str, str] = {}
    for pattern in ("*.ts", "*.vue"):
        for path in sorted(src_root.rglob(pattern)):
            if "__tests__" in path.parts or path.name.endswith(".d.ts"):
                continue
            name = rel(path, src_root.parent)
            source = strip_comments(path.read_text(encoding="utf-8"))
            sources[name] = source
            for match in INVOKE_CALL.finditer(source):
                argument = match.group(1)
                literal_match = STRING_LITERAL.match(argument)
                if literal_match:
                    literal.setdefault(literal_match.group(2), set()).add(name)
                else:
                    dynamic.setdefault(name, []).append(argument)
    return literal, dynamic, sources


def wrapper_invocations(
    wrappers: list[dict[str, str]], sources: dict[str, str], report: Report
) -> tuple[dict[str, set[str]], set[str]]:
    """返回 (字面量命令 -> 文件集合, 允许出现变量转发的文件集合)。"""
    literal: dict[str, set[str]] = {}
    forwarding_files = {wrapper["file"] for wrapper in wrappers}
    for wrapper in wrappers:
        name, home = wrapper["name"], wrapper["file"]
        if home not in sources:
            report.fail(f"invoke_wrappers entry {name} points to missing file {home}")
            continue
        if not re.search(rf"\bfunction\s+{re.escape(name)}\b", sources[home]):
            report.fail(f"invoke_wrappers entry {name} is not defined as a function in {home}")
        call = re.compile(rf"(?<![\w.])(?<!function ){re.escape(name)}\s*{GENERIC_ARGS}\s*\(\s*([^,)\s]+)")
        files = [home] if wrapper.get("scope") == "file" else sorted(sources)
        for file_name in files:
            for match in call.finditer(sources[file_name]):
                argument = match.group(1)
                literal_match = STRING_LITERAL.match(argument)
                if literal_match:
                    literal.setdefault(literal_match.group(2), set()).add(file_name)
                elif file_name not in forwarding_files:
                    report.fail(f"{file_name} calls {name}({argument}) with a non-literal command")
    return literal, forwarding_files


def run(root: Path) -> int:
    config = load_json(root / CONFIG)
    report = Report("IPC contract")
    defined = defined_commands(root / config["commands_dir"])
    registered = registered_commands(root / config["handler_file"])
    if not registered:
        report.fail(f"no generate_handler! list found in {config['handler_file']}")
    duplicates = sorted({entry for entry in registered if registered.count(entry) > 1})
    for entry in duplicates:
        report.fail(f"command {entry} is registered more than once")
    registered_set = set(registered)
    owners: dict[str, list[str]] = {}
    for definition in defined:
        hits = definition.candidates & registered_set
        if not hits:
            report.fail(f"command {definition.display} is defined but not registered in generate_handler!")
        elif len(hits) > 1:
            report.fail(f"command {definition.display} is registered under several paths: {sorted(hits)}")
        for entry in hits:
            owners.setdefault(entry, []).append(definition.display)
    for entry, displays in sorted(owners.items()):
        if len(displays) > 1:
            report.fail(f"generate_handler! entry {entry} matches several definitions: {displays}")
    for entry in sorted(registered_set - set(owners)):
        report.fail(f"generate_handler! entry {entry} has no matching #[tauri::command]")

    names: dict[str, list[str]] = {}
    for entry in registered:
        names.setdefault(entry.split("::")[-1], []).append(entry)
    for name, entries in sorted(names.items()):
        if len(entries) > 1:
            report.fail(f"command name {name} is ambiguous across modules: {entries}")

    literal, dynamic, sources = frontend_invocations(root / config["frontend_root"])
    wrapped, forwarding_files = wrapper_invocations(config.get("invoke_wrappers", []), sources, report)
    for name, files in wrapped.items():
        literal.setdefault(name, set()).update(files)
    for file_name in forwarding_files:
        dynamic.pop(file_name, None)
    for name, files in sorted(literal.items()):
        if name not in names:
            report.fail(f"invoke('{name}') in {sorted(files)} targets an unregistered command")

    declared_dynamic: dict[str, list[str]] = config.get("dynamic_invocations", {})
    referenced = set(literal)
    for file_name, arguments in sorted(dynamic.items()):
        candidates = declared_dynamic.get(file_name)
        if candidates is None:
            report.fail(f"{file_name} calls invoke({', '.join(arguments)}) with a non-literal command; declare candidates in {CONFIG}")
            continue
        for candidate in candidates:
            if candidate not in names:
                report.fail(f"dynamic candidate {candidate} for {file_name} is not a registered command")
            if not re.search(rf"""['"`]{re.escape(candidate)}['"`]""", sources[file_name]):
                report.fail(f"dynamic candidate {candidate} is not a string literal in {file_name}")
            referenced.add(candidate)
    for file_name in sorted(set(declared_dynamic) - set(dynamic)):
        report.fail(f"dynamic_invocations entry {file_name} no longer has a non-literal invoke(); remove it")

    baseline = set(config.get("unreferenced_baseline", []))
    unreferenced = set(names) - referenced
    for name in sorted(unreferenced - baseline):
        report.fail(f"command {name} is registered but never invoked by the frontend; remove it or call it")
    for name in sorted(baseline - unreferenced):
        state = "no longer registered" if name not in names else "now referenced"
        report.fail(f"unreferenced_baseline entry {name} is {state}; remove it from {CONFIG}")

    print(f"IPC contract: {len(defined)} defined, {len(registered)} registered, {len(literal)} invoked by literal")
    return report.emit()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)
    return run(args.root.resolve())


if __name__ == "__main__":
    sys.exit(main())

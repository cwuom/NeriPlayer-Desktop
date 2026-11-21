"""模块边界门禁：检查 Rust workspace crate 的登记、依赖方向、Tauri 隔离与文件容量，
以及前端分层导入、UI 层直接 IPC 与文件容量。

规则定义在 config/quality/module-boundaries.json，不按目录名推导依赖层级。
用法：python3 -B tools/quality/module_boundaries.py [--root <repo>]
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

from quality_common import INVOKE_CALL, REPO_ROOT, Report, count_lines, load_json, rel, strip_comments

CONFIG = Path("config/quality/module-boundaries.json")
# 内联测试模块之后的内容不计入生产代码行数
INLINE_TESTS = re.compile(r"^#\[cfg\(test\)\]\s*\n(?:#\[[^\n]*\]\s*\n)*mod \w+ \{", re.M)
FRONTEND_IMPORT = re.compile(
    r"""(?:\bfrom\s*|\bimport\s*\(\s*|^\s*import\s+)(['"])([^'"]+)\1""", re.M
)


def production_lines(path: Path) -> int:
    if path.name == "tests.rs" or "tests" in path.parent.parts[-1:]:
        return 0
    source = path.read_text(encoding="utf-8")
    match = INLINE_TESTS.search(source)
    if match:
        source = source[: match.start()]
    return source.count("\n") + (0 if source.endswith("\n") or not source else 1)


def dependency_tables(manifest: dict) -> list[dict]:
    tables = [manifest.get(key, {}) for key in ("dependencies", "dev-dependencies", "build-dependencies")]
    for target in manifest.get("target", {}).values():
        tables.extend(target.get(key, {}) for key in ("dependencies", "dev-dependencies", "build-dependencies"))
    return tables


def declared_dependencies(manifest: dict) -> set[str]:
    names: set[str] = set()
    for table in dependency_tables(manifest):
        for name, spec in table.items():
            package = spec.get("package") if isinstance(spec, dict) else None
            names.add(package or name)
    return names


def find_cycle(graph: dict[str, set[str]]) -> list[str] | None:
    visiting: list[str] = []
    done: set[str] = set()

    def visit(node: str) -> list[str] | None:
        if node in visiting:
            return visiting[visiting.index(node):] + [node]
        if node in done:
            return None
        visiting.append(node)
        for nxt in sorted(graph.get(node, ())):
            cycle = visit(nxt)
            if cycle:
                return cycle
        visiting.pop()
        done.add(node)
        return None

    for start in sorted(graph):
        cycle = visit(start)
        if cycle:
            return cycle
    return None


def check_rust(root: Path, config: dict, report: Report) -> None:
    rust = config["rust"]
    crates_dir = root / rust["crates_dir"]
    registered: dict[str, dict] = rust["crates"]
    on_disk = {path.name for path in crates_dir.iterdir() if (path / "Cargo.toml").is_file()} if crates_dir.is_dir() else set()
    for name in sorted(on_disk - registered.keys()):
        report.fail(f"rust: crate {name} exists on disk but is not registered in {CONFIG}")
    for name in sorted(registered.keys() - on_disk):
        report.fail(f"rust: registered crate {name} is missing from {rust['crates_dir']}")

    internal = set(registered) | {rust["app_crate"]}
    graph: dict[str, set[str]] = {}
    forbidden = tuple(rust.get("forbidden_external_prefixes", []))
    for name in sorted(on_disk & registered.keys()):
        crate_dir = crates_dir / name
        manifest = tomllib.loads((crate_dir / "Cargo.toml").read_text(encoding="utf-8"))
        package_name = manifest.get("package", {}).get("name")
        if package_name != name:
            report.fail(f"rust: {name}/Cargo.toml declares package name {package_name!r}")
        deps = declared_dependencies(manifest)
        project_deps = {dep for dep in deps if dep in internal or dep.startswith("neri-")}
        graph[name] = project_deps
        allowed = set(registered[name].get("allowed", []))
        for dep in sorted(project_deps - allowed):
            report.fail(f"rust: {name} must not depend on {dep} (allowed: {sorted(allowed) or 'none'})")
        for dep in sorted(deps):
            if forbidden and dep.startswith(forbidden):
                report.fail(f"rust: domain crate {name} must not depend on {dep}; host adapters belong to the app crate")
        if rust.get("require_readme") and not (crate_dir / "README.md").is_file():
            report.fail(f"rust: {name} is missing README.md")
        for source in sorted(crate_dir.rglob("*.rs")):
            text = source.read_text(encoding="utf-8")
            for prefix in forbidden:
                if re.search(rf"\b{re.escape(prefix.replace('-', '_'))}\w*::", text):
                    report.fail(f"rust: {rel(source, root)} references {prefix} APIs")
                    break

    app_manifest = tomllib.loads((root / rust["workspace"] / "Cargo.toml").read_text(encoding="utf-8"))
    app_deps = {dep for dep in declared_dependencies(app_manifest) if dep.startswith("neri-")}
    for dep in sorted(app_deps - set(registered)):
        report.fail(f"rust: app crate depends on unregistered project crate {dep}")
    graph[rust["app_crate"]] = app_deps
    cycle = find_cycle(graph)
    if cycle:
        report.fail("rust: dependency cycle " + " -> ".join(cycle))

    max_lines = rust["max_production_lines"]
    max_files = rust["max_rs_files_per_directory"]
    source_roots = [crates_dir, root / rust["workspace"] / "src"]
    directories: dict[Path, int] = {}
    for source_root in source_roots:
        for source in sorted(source_root.rglob("*.rs")):
            if "target" in source.relative_to(root).parts:
                continue
            directories[source.parent] = directories.get(source.parent, 0) + 1
            lines = production_lines(source)
            if lines > max_lines:
                report.fail(f"rust: {rel(source, root)} has {lines} production lines (limit {max_lines}); split it by responsibility")
    for directory, count in sorted(directories.items()):
        if count > max_files:
            report.fail(f"rust: {rel(directory, root)} has {count} .rs files (limit {max_files}); introduce a submodule")


def frontend_layer(path: Path, src_root: Path) -> str:
    parts = path.relative_to(src_root).parts
    # 目录入口（如 '@/i18n' 指向 src/i18n/index.ts）也归属该目录对应的层
    if len(parts) > 1 or (parts and (src_root / parts[0]).is_dir()):
        return parts[0]
    return "(root)"


def resolve_import(specifier: str, path: Path, src_root: Path) -> Path | None:
    if specifier.startswith("@/"):
        return src_root / specifier[2:]
    if specifier.startswith("."):
        return (path.parent / specifier).resolve()
    return None


def frontend_sources(src_root: Path) -> list[Path]:
    files = []
    for pattern in ("*.ts", "*.vue"):
        for path in src_root.rglob(pattern):
            if "__tests__" in path.parts or path.name.endswith(".d.ts"):
                continue
            files.append(path)
    return sorted(files)


def check_frontend(root: Path, config: dict, report: Report) -> None:
    frontend = config["frontend"]
    src_root = (root / frontend["root"]).resolve()
    layers: dict[str, list[str] | str] = frontend["layers"]
    max_lines = frontend["max_lines"]
    legacy: dict[str, int] = frontend.get("legacy_oversized", {})
    invoke_layers = set(frontend["ipc_layers"])
    invoke_baseline: dict[str, int] = frontend.get("ui_invoke_baseline", {})
    max_files = frontend["max_files_per_directory"]
    seen_invoke: dict[str, int] = {}
    directories: dict[Path, int] = {}

    for path in frontend_sources(src_root):
        name = rel(path, src_root.parent)
        layer = frontend_layer(path, src_root)
        directories[path.parent] = directories.get(path.parent, 0) + 1
        if layer not in layers:
            report.fail(f"frontend: {name} is in unregistered layer {layer!r}")
            continue
        allowed = layers[layer]
        source = strip_comments(path.read_text(encoding="utf-8"))
        if allowed != "*":
            for match in FRONTEND_IMPORT.finditer(source):
                target = resolve_import(match.group(2), path.resolve(), src_root)
                if target is None:
                    continue
                try:
                    target_layer = frontend_layer(target, src_root)
                except ValueError:
                    report.fail(f"frontend: {name} imports {match.group(2)} outside src/")
                    continue
                if target_layer not in allowed:
                    report.fail(f"frontend: {name} ({layer}) must not import {match.group(2)} ({target_layer})")

        lines = count_lines(path)
        ceiling = legacy.get(name)
        if ceiling is not None:
            if lines > ceiling:
                report.fail(f"frontend: legacy oversized {name} grew to {lines} lines (ceiling {ceiling}); split instead of growing")
            elif lines <= max_lines:
                report.fail(f"frontend: {name} is now {lines} lines; remove it from legacy_oversized")
            elif lines < ceiling:
                report.fail(f"frontend: lower legacy_oversized ceiling for {name} from {ceiling} to {lines}")
        elif lines > max_lines:
            report.fail(f"frontend: {name} has {lines} lines (limit {max_lines}); split it into components or modules")

        if layer not in invoke_layers and layer != "(root)":
            calls = len(INVOKE_CALL.findall(source))
            if calls:
                seen_invoke[name] = calls
            budget = invoke_baseline.get(name, 0)
            if calls > budget:
                report.fail(
                    f"frontend: {name} ({layer}) has {calls} direct invoke() calls (baseline {budget}); "
                    "move IPC into stores or modules"
                )

    for name, budget in sorted(invoke_baseline.items()):
        actual = seen_invoke.get(name, 0)
        if actual < budget:
            report.fail(f"frontend: lower ui_invoke_baseline for {name} from {budget} to {actual}")
    for name in sorted(legacy):
        if not (src_root.parent / name).is_file():
            report.fail(f"frontend: legacy_oversized entry {name} no longer exists")
    crowded: dict[str, int] = frontend.get("legacy_crowded_directories", {})
    for directory, count in sorted(directories.items()):
        name = rel(directory, src_root.parent)
        ceiling = crowded.get(name)
        if ceiling is not None:
            if count > ceiling:
                report.fail(f"frontend: legacy crowded {name} grew to {count} source files (ceiling {ceiling})")
            elif count <= max_files:
                report.fail(f"frontend: {name} now has {count} source files; remove it from legacy_crowded_directories")
            elif count < ceiling:
                report.fail(f"frontend: lower legacy_crowded_directories ceiling for {name} from {ceiling} to {count}")
        elif count > max_files:
            report.fail(f"frontend: {name} has {count} source files (limit {max_files})")
    for name in sorted(crowded):
        if not (src_root.parent / name).is_dir():
            report.fail(f"frontend: legacy_crowded_directories entry {name} no longer exists")


def run(root: Path) -> int:
    config = load_json(root / CONFIG)
    rust_report = Report("Rust module boundaries")
    check_rust(root, config, rust_report)
    frontend_report = Report("Frontend module boundaries")
    check_frontend(root, config, frontend_report)
    return max(rust_report.emit(), frontend_report.emit())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)
    return run(args.root.resolve())


if __name__ == "__main__":
    sys.exit(main())

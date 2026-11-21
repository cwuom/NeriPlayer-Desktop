"""可维护性度量：采集可复现的原始指标，并按公开的评分规则折算为 100 分制。

默认只做静态统计（秒级）。加 `--run` 时实际执行门禁、ESLint、测试与覆盖率命令并把结果计入；
没有执行的项目在报告里标为「未测」并按 0 分计，避免把没跑过的检查算成通过。

用法：
    python3 -B tools/quality/maintainability.py [--run] [--json <输出路径>] [--markdown <输出路径>]

覆盖率需要 `cargo-llvm-cov`（可用 `--cargo-llvm-cov <可执行文件>` 指定）与 Vitest v8 覆盖率。
评分规则写在 RUBRIC 中；调整权重需要在 PR 里说明理由。
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

from quality_common import REPO_ROOT, count_lines, load_json, rel
from module_boundaries import production_lines

RUST_TEST_ATTR = re.compile(r"#\[(?:tokio::)?test\b")
TS_ANY = re.compile(r"(?::\s*any\b|\bas\s+any\b|<any\b|\bany\[\]|,\s*any\s*>|\bany\s*\|)")
VITEST_CASE = re.compile(r"^\s*(?:it|test)(?:\.each\([^)]*\))?\s*\(", re.M)
FRONTEND_MAX_LINES = 2000
RUST_MAX_LINES = 2000


@dataclass
class Metric:
    key: str
    value: float | int | str | None
    note: str = ""


@dataclass
class Score:
    area: str
    item: str
    points: float
    max_points: float
    basis: str


@dataclass
class Result:
    metrics: dict[str, Metric] = field(default_factory=dict)
    scores: list[Score] = field(default_factory=list)

    def put(self, key: str, value: float | int | str | None, note: str = "") -> None:
        self.metrics[key] = Metric(key, value, note)

    def get(self, key: str) -> float | int | str | None:
        metric = self.metrics.get(key)
        return None if metric is None else metric.value


# ---------- 静态指标 ----------


def rust_sources(root: Path) -> list[Path]:
    base = root / "src-tauri"
    return sorted(
        path
        for path in base.rglob("*.rs")
        if "target" not in path.relative_to(base).parts and "fixtures" not in path.parts
    )


def frontend_sources(root: Path) -> list[Path]:
    src = root / "src"
    return sorted(
        path
        for pattern in ("*.ts", "*.vue")
        for path in src.rglob(pattern)
        if "__tests__" not in path.parts and not path.name.endswith(".d.ts")
    )


def collect_static(root: Path, result: Result) -> None:
    crates_dir = root / "src-tauri" / "crates"
    crates = sorted(path for path in crates_dir.iterdir() if (path / "Cargo.toml").is_file()) if crates_dir.is_dir() else []
    result.put("rust.crates", len(crates))
    result.put("rust.crates_with_readme", sum((crate / "README.md").is_file() for crate in crates))
    tauri_free = 0
    for crate in crates:
        manifest = (crate / "Cargo.toml").read_text(encoding="utf-8")
        sources = "".join(path.read_text(encoding="utf-8") for path in (crate / "src").rglob("*.rs"))
        if not re.search(r"^\s*tauri", manifest, re.M) and "tauri::" not in sources:
            tauri_free += 1
    result.put("rust.tauri_free_crates", tauri_free)

    sources = rust_sources(root)
    production = {rel(path, root): production_lines(path) for path in sources}
    result.put("rust.files", len(sources))
    result.put("rust.production_lines", sum(production.values()))
    result.put("rust.max_file_lines", max(production.values(), default=0),
               max(production, key=production.get) if production else "")
    result.put("rust.oversized_files", sum(lines > RUST_MAX_LINES for lines in production.values()))
    result.put("rust.test_functions", sum(len(RUST_TEST_ATTR.findall(path.read_text(encoding="utf-8"))) for path in sources))

    files = frontend_sources(root)
    lines = {rel(path, root): count_lines(path) for path in files}
    result.put("frontend.files", len(files))
    result.put("frontend.lines", sum(lines.values()))
    result.put("frontend.max_file_lines", max(lines.values(), default=0), max(lines, key=lines.get) if lines else "")
    result.put("frontend.oversized_files", sum(count > FRONTEND_MAX_LINES for count in lines.values()))
    any_count = 0
    for path in files:
        text = re.sub(r"/\*.*?\*/|//[^\n]*", "", path.read_text(encoding="utf-8"), flags=re.S)
        any_count += len(TS_ANY.findall(text))
    result.put("frontend.any_annotations", any_count, "源码中的 any 标注（不含测试）")

    unit_tests = sorted((root / "src").rglob("__tests__/*.test.ts"))
    result.put("frontend.vitest_files", len(unit_tests))
    result.put("frontend.vitest_cases", sum(len(VITEST_CASE.findall(path.read_text(encoding="utf-8"))) for path in unit_tests))
    store_files = {path.stem for path in (root / "src" / "stores").glob("*.ts")}
    store_files |= {path.name for path in (root / "src" / "stores").iterdir() if path.is_dir() and path.name != "__tests__"}
    tested = {path.name.split(".")[0] for path in (root / "src" / "stores").rglob("__tests__/*.test.ts")}
    result.put("frontend.stores", len(store_files))
    result.put("frontend.stores_with_unit_tests", len(store_files & tested))
    package = load_json(root / "package.json")
    result.put("frontend.regression_scripts", sum(name.startswith("test:") for name in package.get("scripts", {})))

    docs = ["README.md", "README_EN.md", "CONTRIBUTING.md", "CONTRIBUTING_EN.md", "docs/implementation.md",
            "docs/implementation_EN.md", "SECURITY.md", "tools/quality/README.md", "tools/quality/README_EN.md"]
    result.put("docs.present", sum((root / doc).is_file() for doc in docs), f"/{len(docs)}")
    templates = [".github/ISSUE_TEMPLATE/bug-report.yml", ".github/ISSUE_TEMPLATE/feature-request.yml",
                 ".github/ISSUE_TEMPLATE/config.yml", ".github/PULL_REQUEST_TEMPLATE.md",
                 ".github/workflows/issue_labels.yml", ".github/scripts/issue_labels.test.cjs", ".github/labels.json"]
    result.put("process.triage_files", sum((root / item).is_file() for item in templates), f"/{len(templates)}")
    workflows = "\n".join(path.read_text(encoding="utf-8") for path in (root / ".github" / "workflows").glob("*.yml"))
    ci_checks = {
        "typecheck": "pnpm typecheck",
        "lint": "pnpm lint",
        "frontend-tests": "pnpm test",
        "frontend-coverage": "unit:coverage",
        "quality-gates": "module_boundaries.py",
        "ipc-contract": "ipc_contract.py",
        "rust-fmt": "cargo fmt --all --check",
        "rust-clippy-release": "--release -- -D warnings",
        "rust-tests": "cargo llvm-cov",
    }
    present = [name for name, needle in ci_checks.items() if needle in workflows]
    result.put("process.ci_checks", len(present), f"/{len(ci_checks)}: {', '.join(present)}")


# ---------- 执行型指标 ----------


def run(command: list[str], cwd: Path, timeout: int) -> tuple[int, str]:
    started = time.monotonic()
    try:
        completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        return 127, str(error)
    print(f"  {' '.join(command)} -> {completed.returncode} ({time.monotonic() - started:.0f}s)", file=sys.stderr)
    return completed.returncode, completed.stdout + completed.stderr


def collect_dynamic(root: Path, result: Result, cargo_llvm_cov: str) -> None:
    tauri = root / "src-tauri"
    code, _ = run([sys.executable, "-B", "tools/quality/module_boundaries.py"], root, 120)
    result.put("gate.module_boundaries", code == 0)
    code, _ = run([sys.executable, "-B", "tools/quality/ipc_contract.py"], root, 120)
    result.put("gate.ipc_contract", code == 0)

    code, output = run(["npx", "eslint", ".", "-f", "json"], root, 900)
    try:
        report = json.loads(output[output.index("["):])
        result.put("frontend.eslint_errors", sum(item["errorCount"] for item in report))
        result.put("frontend.eslint_warnings", sum(item["warningCount"] for item in report))
    except ValueError:
        result.put("frontend.eslint_errors", None, "ESLint 输出无法解析")
    code, _ = run(["pnpm", "typecheck"], root, 900)
    result.put("frontend.typecheck", code == 0)
    code, output = run(["node", "scripts/test-frontend.mjs"], root, 1800)
    result.put("frontend.regression_pass", code == 0, tail(output))
    code, output = run(["npx", "vitest", "run", "--coverage", "--coverage.reporter=json-summary",
                        "--coverage.reporter=json"], root, 1800)
    result.put("frontend.vitest_pass", code == 0, tail(output))
    summary = root / "coverage" / "coverage-summary.json"
    if summary.is_file():
        total = load_json(summary)["total"]
        result.put("frontend.coverage_lines", total["lines"]["pct"], "modules/utils/stores 行覆盖率 %")
    # 与 Android 仓库同款的 CRAP 门禁，只作为原始指标展示，不改变评分规则
    code, output = run([sys.executable, "-B", "tools/quality/crap_report.py"], root, 600)
    result.put("crap.gate", code == 0, tail(output, 2))
    ratio_match = re.search(r"CRAP: (\d+)/(\d+) functions <= 9", output)
    if ratio_match:
        passing, total_functions = int(ratio_match.group(1)), int(ratio_match.group(2))
        result.put("crap.functions_at_most_9", round(passing / total_functions * 100, 2),
                   f"{passing}/{total_functions} 个函数 CRAP ≤ 9（%）")

    for profile, extra in (("debug", []), ("release", ["--release"])):
        code, _ = run(["cargo", "clippy", "--workspace", "--all-targets", *extra, "--", "-D", "warnings"], tauri, 3600)
        result.put(f"rust.clippy_{profile}", code == 0)
    code, _ = run(["cargo", "fmt", "--all", "--check"], tauri, 300)
    result.put("rust.fmt", code == 0)
    output_path = tauri / "target" / "llvm-cov-summary.json"
    code, output = run([cargo_llvm_cov, "llvm-cov", "--workspace", "--json", "--summary-only",
                        "--output-path", str(output_path)], tauri, 5400)
    result.put("rust.tests_pass", code == 0, tail(output))
    if code == 0 and output_path.is_file():
        totals = load_json(output_path)["data"][0]["totals"]
        result.put("rust.coverage_lines", round(totals["lines"]["percent"], 2), "workspace 行覆盖率 %")


def tail(output: str, lines: int = 3) -> str:
    return " | ".join(line.strip() for line in output.strip().splitlines()[-lines:])


# ---------- 评分 ----------


def ratio(value: float | int | str | None, target: float) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool) or target <= 0:
        return 0.0
    return max(0.0, min(1.0, value / target))


def flag(value: float | int | str | None) -> float:
    return 1.0 if value is True else 0.0


def decay(value: float | int | str | None, zero_at: float) -> float:
    """value 为 0 得满分，达到 zero_at 时为 0；未测量按 0 分计。"""
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return 0.0
    return max(0.0, 1.0 - value / zero_at)


def score(result: Result) -> None:
    g = result.get
    crates = g("rust.crates") or 0
    rubric = [
        ("模块化", "Rust 领域 crate ≥ 5 且全部不依赖 Tauri", 6, min(ratio(crates, 5), ratio(g("rust.tauri_free_crates"), max(crates, 1)))),
        ("模块化", "模块边界门禁通过", 6, flag(g("gate.module_boundaries"))),
        ("模块化", "IPC 契约门禁通过", 4, flag(g("gate.ipc_contract"))),
        ("模块化", "超 2000 行的 Rust 文件为 0（每个扣 1/3）", 4, decay(g("rust.oversized_files"), 3)),
        ("模块化", "超 2000 行的前端文件为 0（每个扣 1/3）", 5, decay(g("frontend.oversized_files"), 3)),
        ("类型与静态检查", "clippy -D warnings（debug）", 3, flag(g("rust.clippy_debug"))),
        ("类型与静态检查", "clippy -D warnings（release）", 3, flag(g("rust.clippy_release"))),
        ("类型与静态检查", "rustfmt 一致", 2, flag(g("rust.fmt"))),
        ("类型与静态检查", "vue-tsc 通过", 3, flag(g("frontend.typecheck"))),
        ("类型与静态检查", "ESLint error 为 0（每个扣 1/10）", 4, decay(g("frontend.eslint_errors"), 10)),
        ("类型与静态检查", "any 标注数（100 个时为 0 分）", 5, decay(g("frontend.any_annotations"), 100)),
        ("测试", "Rust 全量测试通过", 5, flag(g("rust.tests_pass"))),
        ("测试", "Rust 行覆盖率（目标 70%）", 8, ratio(g("rust.coverage_lines"), 70)),
        ("测试", "前端回归脚本全部通过", 4, flag(g("frontend.regression_pass"))),
        ("测试", "Vitest 全部通过", 3, flag(g("frontend.vitest_pass"))),
        ("测试", "Vitest 行覆盖率（目标 70%）", 6, ratio(g("frontend.coverage_lines"), 70)),
        ("测试", "Pinia store 有 Vitest 单测的比例（目标 50%）", 4, ratio((g("frontend.stores_with_unit_tests") or 0) / max(g("frontend.stores") or 1, 1), 0.5)),
        ("文档", "中英 README / CONTRIBUTING / 实现概览 / 门禁说明 / SECURITY 齐全", 7, ratio(g("docs.present"), 9)),
        ("文档", "每个 crate 有 README", 5, ratio(g("rust.crates_with_readme"), max(crates, 1))),
        ("流程", "CI 覆盖类型检查、lint、测试、覆盖率、门禁、fmt、clippy", 8, ratio(g("process.ci_checks"), 9)),
        ("流程", "Issue 表单、标签分流与 PR 模板齐全且有测试", 5, ratio(g("process.triage_files"), 7)),
    ]
    for area, item, points, fraction in rubric:
        result.scores.append(Score(area, item, round(points * fraction, 2), points, f"{fraction:.2f}"))


# ---------- 输出 ----------


def markdown(result: Result) -> str:
    total = sum(item.points for item in result.scores)
    maximum = sum(item.max_points for item in result.scores)
    lines = ["# 可维护性度量", "", f"总分：**{total:.1f} / {maximum:.0f}**", "",
             "## 原始指标", "", "| 指标 | 值 | 说明 |", "| --- | --- | --- |"]
    for metric in result.metrics.values():
        value = "未测" if metric.value is None else metric.value
        lines.append(f"| `{metric.key}` | {value} | {metric.note} |")
    lines += ["", "## 评分", "", "| 领域 | 项目 | 得分 | 满分 |", "| --- | --- | --- | --- |"]
    for item in result.scores:
        lines.append(f"| {item.area} | {item.item} | {item.points} | {item.max_points} |")
    lines += ["", "未执行 `--run` 时，执行型指标记为「未测」并按 0 分计。"]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    parser.add_argument("--run", action="store_true", help="执行门禁、lint、测试与覆盖率命令")
    parser.add_argument("--cargo-llvm-cov", default="cargo-llvm-cov")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--markdown", type=Path)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    result = Result()
    collect_static(root, result)
    if args.run:
        collect_dynamic(root, result, args.cargo_llvm_cov)
    score(result)
    text = markdown(result)
    print(text)
    if args.markdown:
        args.markdown.write_text(text, encoding="utf-8")
    if args.json:
        payload = {
            "metrics": {key: metric.value for key, metric in result.metrics.items()},
            "scores": [vars(item) for item in result.scores],
            "total": sum(item.points for item in result.scores),
        }
        args.json.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())

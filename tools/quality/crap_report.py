"""CRAP 门禁：对齐 Android 仓库 tools_pub/quality/crap_report.py 的公式与阈值。

CRAP = C² × (1 − cov)³ + C
- C：函数圈复杂度，由 ts_complexity.mjs 按 ESLint 内置 complexity 规则的计数点统计
- cov：函数自身（不含嵌套函数）的语句覆盖率与 v8 块覆盖率中较低者，来自 Vitest 的 istanbul JSON；
  它是路径覆盖率的近似，和 Android 用 JaCoCo 复杂度覆盖率近似路径覆盖率是同一思路
- 受检范围（source_patterns）内任一函数 CRAP > 9 即失败；范围只增不减
- 范围外 CRAP > 9 的函数数记在 unscoped_failure_baseline，只能下降（与其它门禁的基线一致）
- 所有统计范围内 CRAP > 8 的函数列入报告

用法：
    pnpm unit:coverage   # 生成 coverage/coverage-final.json
    python3 -B tools/quality/crap_report.py [--coverage <json>] [--output <dir>] [--report-only]
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import subprocess
import sys
from pathlib import Path

from quality_common import REPO_ROOT, load_json

REPORT_THRESHOLD = 8
FAIL_THRESHOLD = 9
CONFIG = Path("config/quality/crap-scope.json")


def crap(complexity: int, coverage: float) -> float:
    return complexity * complexity * (1 - coverage) ** 3 + complexity


def matches(path: str, patterns: list[str]) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def measured_files(root: Path, config: dict) -> list[str]:
    files = set()
    for pattern in config["measure_patterns"]:
        for path in root.glob(pattern):
            name = path.relative_to(root).as_posix()
            if path.is_file() and "__tests__" not in path.parts and not name.endswith(".d.ts"):
                files.add(name)
    return sorted(files)


def validate_scope(files: list[str], config: dict) -> None:
    patterns = config["source_patterns"]
    if not isinstance(patterns, list) or not patterns:
        raise ValueError("CRAP scope must contain source_patterns")
    for pattern in patterns:
        if not any(fnmatch.fnmatchcase(name, pattern) for name in files):
            raise ValueError(f"Scope pattern matches no measured sources: {pattern}")


def complexity_index(root: Path, files: list[str]) -> dict[str, list[dict]]:
    completed = subprocess.run(
        ["node", str(root / "tools/quality/ts_complexity.mjs"), *files],
        cwd=root, capture_output=True, text=True, check=True,
    )
    return {item["file"]: item["functions"] for item in json.loads(completed.stdout)}


def innermost(functions: list[dict], line: int) -> int | None:
    best = None
    for index, function in enumerate(functions):
        if function["start"]["line"] <= line <= function["end"]["line"]:
            if best is None or (function["start"]["line"], function["start"]["column"]) >= (
                functions[best]["start"]["line"], functions[best]["start"]["column"]
            ):
                best = index
    return best


def function_coverage(functions: list[dict], file_coverage: dict | None) -> list[float]:
    """每个函数自身的语句覆盖率与块覆盖率取较低者；没有覆盖数据的函数按 0 计。"""
    statements = [[0, 0] for _ in functions]
    blocks = [[0, 0] for _ in functions]
    if file_coverage:
        for key, location in file_coverage.get("statementMap", {}).items():
            owner = innermost(functions, location["start"]["line"])
            if owner is not None:
                statements[owner][1] += 1
                statements[owner][0] += file_coverage["s"].get(key, 0) > 0
        for key, branch in file_coverage.get("branchMap", {}).items():
            owner = innermost(functions, branch["loc"]["start"]["line"])
            if owner is not None:
                counts = file_coverage["b"].get(key, [])
                blocks[owner][1] += len(counts)
                blocks[owner][0] += sum(count > 0 for count in counts)
    result = []
    for (covered, total), (block_covered, block_total) in zip(statements, blocks):
        ratios = [covered / total] if total else []
        if block_total:
            ratios.append(block_covered / block_total)
        result.append(min(ratios) if ratios else 0.0)
    return result


def collect(root: Path, config: dict, coverage: dict, complexities: dict[str, list[dict]]) -> list[dict]:
    by_file = {Path(path).resolve().relative_to(root.resolve()).as_posix(): data
               for path, data in coverage.items() if Path(path).resolve().is_relative_to(root.resolve())}
    rows = []
    for name, functions in sorted(complexities.items()):
        ratios = function_coverage(functions, by_file.get(name))
        for function, ratio in zip(functions, ratios):
            rows.append({
                "source": name,
                "line": function["start"]["line"],
                "function": function["name"],
                "complexity": function["complexity"],
                "coverage": ratio,
                "crap": crap(function["complexity"], ratio),
                "in_scope": matches(name, config["source_patterns"]),
            })
    return sorted(rows, key=lambda row: (-row["crap"], row["source"], row["line"]))


def markdown(title: str, rows: list[dict]) -> str:
    lines = [f"# {title}", "", "CRAP = C² × (1 − cov)³ + C; C = ESLint complexity; "
             "cov = min(own statement coverage, own v8 block coverage).", "",
             "| Source | Function | C | Coverage | CRAP | Gate scope |", "| --- | --- | ---: | ---: | ---: | --- |"]
    for row in rows:
        lines.append(f"| {row['source']}:{row['line']} | `{row['function']}` | {row['complexity']} | "
                     f"{row['coverage']:.2%} | {row['crap']:.6f} | {'yes' if row['in_scope'] else 'no'} |")
    return "\n".join(lines) + "\n"


def run(args: argparse.Namespace) -> int:
    root = args.root.resolve()
    config = load_json(root / CONFIG)
    files = measured_files(root, config)
    validate_scope(files, config)
    complexities = load_json(args.complexity) if args.complexity else complexity_index(root, files)
    rows = collect(root, config, load_json(args.coverage), complexities)
    scoped = [row for row in rows if row["in_scope"]]
    listed = [row for row in rows if row["crap"] > REPORT_THRESHOLD]
    failures = [row for row in scoped if row["crap"] > FAIL_THRESHOLD]
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "functions.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    (args.output / "above-8.md").write_text(markdown("CRAP > 8", listed), encoding="utf-8")
    (args.output / "scope.md").write_text(markdown("All scoped functions", scoped), encoding="utf-8")
    unscoped = sum(1 for row in rows if not row["in_scope"] and row["crap"] > FAIL_THRESHOLD)
    baseline = config["unscoped_failure_baseline"]
    passing = sum(1 for row in rows if row["crap"] <= FAIL_THRESHOLD)
    print(f"CRAP: {len(rows)} functions, {len(listed)} > 8; {len(scoped)} scoped functions, {len(failures)} > 9")
    print(f"CRAP: {passing}/{len(rows)} functions <= 9; {unscoped} unscoped functions > 9 (baseline {baseline})")
    print(f"Reports: {args.output}")
    for row in failures:
        print(f"FAIL {row['source']}:{row['line']} {row['function']} C={row['complexity']} "
              f"cov={row['coverage']:.2%} CRAP={row['crap']:.6f}")
    ratchet = False
    if unscoped > baseline:
        ratchet = True
        print(f"FAIL {unscoped} unscoped functions exceed CRAP 9, baseline is {baseline}; "
              f"add tests or simplify the new code (see {args.output / 'above-8.md'})")
    elif unscoped < baseline:
        ratchet = True
        print(f"FAIL lower unscoped_failure_baseline in {CONFIG} from {baseline} to {unscoped}")
    return 1 if (failures or ratchet) and not args.report_only else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    parser.add_argument("--coverage", type=Path, default=REPO_ROOT / "coverage" / "coverage-final.json")
    parser.add_argument("--complexity", type=Path, help="预先生成的 ts_complexity.mjs 输出（测试用）")
    parser.add_argument("--output", type=Path, default=REPO_ROOT / "coverage" / "crap")
    parser.add_argument("--report-only", action="store_true")
    args = parser.parse_args(argv)
    try:
        return run(args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"CRAP report error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())

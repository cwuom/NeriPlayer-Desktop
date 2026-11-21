"""质量门禁脚本共用的小工具：仓库定位、配置读取与违规收集。只使用 Python 标准库。"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# 前端 IPC 调用：invoke('cmd') / invoke<T>('cmd') / invoke<Map<K, V>>('cmd')
# 泛型参数最多嵌套三层，例如 invoke<Array<Record<string, unknown> | null>>(...)
GENERIC_ARGS = r"(?:<(?:[^<>]|<(?:[^<>]|<[^<>]*>)*>)*>)?"
INVOKE_CALL = re.compile(r"\binvoke\s*" + GENERIC_ARGS + r"\s*\(\s*([^,)\s]+)")
STRING_LITERAL = re.compile(r"""^(['"`])(\w+)\1$""")


@dataclass
class Report:
    """按检查项收集违规；任一违规即门禁失败。"""

    title: str
    violations: list[str] = field(default_factory=list)

    def fail(self, message: str) -> None:
        self.violations.append(message)

    def emit(self) -> int:
        if not self.violations:
            print(f"{self.title}: OK")
            return 0
        print(f"{self.title}: {len(self.violations)} violation(s)")
        for item in self.violations:
            print(f"  - {item}")
        return 1


def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def rel(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def count_lines(path: Path) -> int:
    with path.open(encoding="utf-8") as handle:
        return sum(1 for _ in handle)


def strip_comments(source: str) -> str:
    """去掉 JS/TS 注释，避免注释中的示例代码被当成真实调用或导入。"""
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return re.sub(r"(^|[^:])//[^\n]*", r"\1", source)

[English](./README_EN.md) | [中文](./README.md)

# 质量检查

这些工具检查模块边界、IPC 契约、文件容量与函数风险分（CRAP），由 CI 的 `Quality gates` 与前端作业运行。

- 只需 Python 3.11+（使用标准库 `tomllib`），无第三方依赖
- 规则与基线放在 [`config/quality/`](../../config/quality/)，不在脚本里硬编码
- 模块职责与依赖方向见 [实现概览](../../docs/implementation.md#依赖规则)

| 检查 | 入口 | 用途 |
| --- | --- | --- |
| 模块边界 | `module_boundaries.py` | Rust crate 登记与依赖方向、Tauri 隔离、文件与目录容量；前端分层导入、UI 层 IPC 基线、文件容量 |
| IPC 契约 | `ipc_contract.py` | `#[tauri::command]` 定义、`generate_handler!` 注册与前端 `invoke()` 三方一致 |
| CRAP | `crap_report.py`、`ts_complexity.mjs` | 结合函数复杂度与 Vitest 覆盖率检查风险分，公式与阈值对齐 Android 仓库 |
| 可维护性度量 | `maintainability.py` | 汇总上述检查与测试、覆盖率、文档、CI 的原始指标并按公开规则评分 |

## 运行

在仓库根目录运行：

```bash
python3 -B tools/quality/module_boundaries.py
python3 -B tools/quality/ipc_contract.py
python3 -B -m unittest discover -s tools/quality -p 'test_*.py'
node --test tools/quality/*.test.mjs
pnpm crap   # 先生成 Vitest 覆盖率，再运行 crap_report.py
```

返回码：`0` 通过，`1` 存在违规（逐条列出原因与修复方向），`2` 输入或配置错误。

## 模块边界

配置：[`module-boundaries.json`](../../config/quality/module-boundaries.json)

### Rust

| 规则 | 说明 |
| --- | --- |
| crate 登记 | `src-tauri/crates/` 下的每个 crate 都必须登记，包名与目录名一致，且必须有 `README.md` |
| 依赖白名单 | 每个 crate 只能依赖 `allowed` 中的项目 crate；`dependencies`、`dev-dependencies`、`build-dependencies` 与 `target.*` 表都会检查 |
| 无循环 | 包含应用壳层在内的项目依赖图不得有环 |
| Tauri 隔离 | 领域 crate 不得依赖 `tauri*`，源码不得出现 `tauri::` / `tauri_plugin_*::` 调用 |
| 文件容量 | 单个 `.rs` 文件的生产代码 ≤ 2000 行；内联 `#[cfg(test)] mod tests` 之后的内容与 `tests.rs` 不计入 |
| 目录容量 | 单个目录直接包含的 `.rs` 文件 ≤ 16 个，超过时引入子模块 |

### 前端

| 规则 | 说明 |
| --- | --- |
| 分层导入 | 按 `layers` 白名单检查 `@/…`、相对路径与动态 `import()`；注释中的导入不计 |
| 未登记层 | `src/` 下新增顶层目录必须先在配置中登记 |
| 文件容量 | 单个 `.ts` / `.vue` ≤ 2000 行；历史超大文件记录在 `legacy_oversized`，只能变小 |
| UI 层 IPC | `components` / `views` 中的直接 `invoke()` 只允许出现在 `ui_invoke_baseline`，次数只能减少 |
| 目录容量 | 单个目录直接包含的源文件 ≤ 24 个；历史超额目录记录在 `legacy_crowded_directories`，文件数只能减少 |

测试文件（`__tests__/`）与声明文件（`*.d.ts`）不参与前端检查。

## IPC 契约

配置：[`ipc-contract.json`](../../config/quality/ipc-contract.json)

- 每个 `#[tauri::command]` 都必须在 `main.rs` 注册，注册项必须有对应定义，且不得重复
- 前端以字面量调用的命令名必须已注册
- 变量形式的调用（如 `invoke(command)`）必须在 `dynamic_invocations` 声明候选，且候选名必须以字面量出现在该文件
- 转发命令名的封装函数（如 `persistUserData`）登记在 `invoke_wrappers`：调用处的字面量命令名计入引用；`scope: "file"` 只统计定义文件内的调用；只有封装所在文件可以继续转发变量命令名
- 前端从未引用的命令只允许出现在 `unreferenced_baseline`；基线条目被重新引用或删除后必须移出

## CRAP

配置：[`crap-scope.json`](../../config/quality/crap-scope.json)。公式与阈值和 Android 仓库 `tools_pub/quality/crap_report.py` 相同：

```text
CRAP = C² × (1 − cov)³ + C
```

| 指标 | 定义 |
| --- | --- |
| `C` | 函数圈复杂度，`ts_complexity.mjs` 按 ESLint 内置 `complexity` 规则的计数点统计（测试会与 ESLint 逐函数比对） |
| `cov` | 函数自身（不含嵌套函数）的语句覆盖率与 v8 块覆盖率中较低者，来自 `coverage/coverage-final.json` |
| 满覆盖 | `cov = 100%` 时 `CRAP = C` |

- `measure_patterns`：统计范围（`src/modules`、`src/utils`、`src/stores` 的 `.ts`），所有 CRAP > 8 的函数写入 `coverage/crap/above-8.md`
- `source_patterns`：受检范围，其中任一函数 CRAP > 9 即失败；只能增加，新增文件达标后加入
- `unscoped_failure_baseline`：受检范围外 CRAP > 9 的函数数，只能下降
- `cov` 是路径覆盖率的近似；`scripts/test-*.mjs` 回归脚本不计入 v8 覆盖率，Vue 组件与 Rust 不在统计范围

## 基线（ratchet）

`legacy_oversized`、`legacy_crowded_directories`、`ui_invoke_baseline`、`unreferenced_baseline` 与 `unscoped_failure_baseline` 记录历史技术债：

- 新文件、新调用或新命令不能进入基线
- 代码改善后，检查会要求同步收紧基线，避免基线失真
- 禁止仅为通过检查而放宽阈值、扩大基线或删除有效测试；确需调整时在 PR 中说明理由

## 工具回归

`test_module_boundaries.py`、`test_ipc_contract.py`、`test_crap_report.py` 与 `test_maintainability.py` 在临时目录构造夹具，`ts_complexity.test.mjs` 与 ESLint 比对复杂度，验证每类违规都会被拒绝、
合规结构能通过。它们验证门禁工具本身，不替代业务测试。

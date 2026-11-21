## 变更说明 / Description

<!-- 简要说明本次变更的目的、实现方式以及解决的问题 -->
<!-- Briefly describe the purpose, implementation, and problem solved by this change -->

## 关联问题 / Related issues

<!-- 使用 Closes #123 或 Fixes #123 自动关联并关闭 Issue -->
<!-- Use Closes #123 or Fixes #123 to link and close an issue automatically -->

## 变更类型 / Type

- [ ] Bug 修复 / Bug fix
- [ ] 新功能 / New feature
- [ ] 体验优化 / Improvement
- [ ] 重构 / Refactor
- [ ] 文档更新 / Documentation
- [ ] 构建或 CI / Build or CI
- [ ] 依赖更新 / Dependency update
- [ ] 其他 / Other

## 影响范围 / Impact

- [ ] Vue 前端 / Vue frontend (`src/`)
- [ ] 应用壳层与 IPC 命令 / App shell and IPC commands (`src-tauri/src/`)
- [ ] 领域 crate / Domain crates (`src-tauri/crates/neri-*`)
- [ ] 音频播放 / Audio playback
- [ ] 平台 API / Platform API
- [ ] 下载或本地音乐 / Downloads or local music
- [ ] 云同步或一起听协议 / Cloud sync or Listen Together protocol
- [ ] 质量门禁或自动化 / Quality gates or automation (`tools/`, `config/`, `.github/`)
- [ ] Windows
- [ ] macOS
- [ ] Linux

## 验证方式 / Verification

<!-- 勾选已运行的检查，并补充手动测试场景与结果；未执行的检查请说明原因 -->
<!-- Tick the checks you ran and add manual scenarios and results; explain any checks not run -->

- [ ] `pnpm typecheck`
- [ ] `pnpm lint`
- [ ] `pnpm test`
- [ ] `pnpm build`
- [ ] `cargo fmt --all --check`（在 `src-tauri/` 下 / in `src-tauri/`）
- [ ] `cargo clippy --workspace --all-targets [--release] -- -D warnings`（debug 与 release 都要 / both debug and release）
- [ ] `cargo test --workspace`
- [ ] `python3 -B tools/quality/module_boundaries.py` 与 / and `python3 -B tools/quality/ipc_contract.py`
- [ ] 已在受影响的桌面平台上手动验证 / Manually tested on affected desktop platforms

## 界面变更 / UI changes

<!-- 涉及界面时请附截图或录屏，否则填写不适用；所有状态变化需要过渡动画 -->
<!-- Add screenshots or recordings for UI changes, otherwise write N/A; every state change needs a transition -->

## 提交前检查 / Checklist

- [ ] 我已阅读并遵循贡献指南 / I have read and followed the contribution guidelines
- [ ] 变更范围聚焦，没有包含无关修改 / The change is focused and contains no unrelated modifications
- [ ] 行为变更或 Bug 修复已补充测试 / Behavior changes and bug fixes include tests
- [ ] 本次更改没有破坏原有行为，并保持向下兼容（如适用） / This change does not break existing behavior and stays backward compatible (if applicable)
- [ ] 我已更新相关文档或注释（如适用） / I updated related documentation or comments when applicable
- [ ] 涉及 IPC 时，已同步命令注册、前端调用与类型 / IPC changes keep command registration, frontend calls, and types aligned
- [ ] 新代码遵循模块依赖方向，未放宽门禁基线 / New code follows module dependency rules without loosening quality baselines
- [ ] 涉及 ProtoBuf、云同步或一起听协议时，已确认与 Android 端兼容 / ProtoBuf, cloud sync, or Listen Together changes stay compatible with Android
- [ ] 我已考虑 Windows、macOS 和 Linux 的兼容性影响 / I considered compatibility across Windows, macOS, and Linux
- [ ] 没有提交 Cookie、Token、密钥或其他敏感信息 / No cookies, tokens, keys, or other secrets are committed

## 补充信息 / Additional information

<!-- 补充审查所需的日志、性能数据、兼容性说明或其他上下文 -->
<!-- Add logs, performance data, compatibility notes, or other review context -->

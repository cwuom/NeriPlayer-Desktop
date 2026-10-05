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

- [ ] Vue 前端 / Vue frontend
- [ ] Rust 或 Tauri 后端 / Rust or Tauri backend
- [ ] 音频播放 / Audio playback
- [ ] 平台 API / Platform API
- [ ] 下载或本地音乐 / Downloads or local music
- [ ] 云同步或一起听协议 / Cloud sync or Listen Together protocol
- [ ] Windows
- [ ] macOS
- [ ] Linux

## 验证方式 / Verification

<!-- 列出已运行的命令、测试场景和结果，未执行的检查请说明原因 -->
<!-- List commands, test scenarios, and results, and explain any checks not run -->

- [ ] `pnpm build`
- [ ] `pnpm test`
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml --locked`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib`
- [ ] 已验证受影响的桌面平台 / Tested affected desktop platforms

## 界面变更 / UI changes

<!-- 涉及界面时请附截图或录屏，否则填写不适用 -->
<!-- Add screenshots or recordings for UI changes, otherwise write N/A -->

## 提交前检查 / Checklist

- [ ] 我已确认变更范围聚焦，没有包含无关修改 / The change is focused and contains no unrelated modifications
- [ ] 我已更新相关文档或注释（如适用） / I updated related documentation or comments when applicable
- [ ] 我已检查 Windows、macOS 和 Linux 的兼容性影响 / I considered compatibility across Windows, macOS, and Linux
- [ ] 涉及 IPC 时，我已同步前后端命令、参数和类型 / I kept frontend and backend IPC commands, arguments, and types aligned
- [ ] 涉及 ProtoBuf、云同步或一起听协议时，我已确认与 Android 端兼容 / I verified Android compatibility for ProtoBuf, cloud sync, or Listen Together changes
- [ ] 我确认没有提交 Cookie、Token、密钥或其他敏感信息 / I committed no cookies, tokens, keys, or other secrets

## 补充信息 / Additional information

<!-- 补充审查所需的日志、性能数据、兼容性说明或其他上下文 -->
<!-- Add logs, performance data, compatibility notes, or other review context -->

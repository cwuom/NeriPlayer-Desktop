# YouTube Challenge Solver

本目录保存 YouTube `sig` 和 `n` challenge 解算所需的两个 JavaScript 资源，由 Rust 嵌入并交给 QuickJS 执行。

## 来源与完整性 / Provenance and Integrity

资源原样取自 Android NeriPlayer 提交 `3e1abcb704a76a3cd211878c7303d4058866c4d4` 的 `modules/platform/src/main/assets/youtube/`。更新时应记录新的来源提交，并重新核对文件哈希。

| 文件 | SHA-256 |
| --- | --- |
| yt.solver.core.min.js | 333f68fff3afa5761db121a7af203dd61da8215cc1f58ffb6e4f3df63da224c5 |
| yt.solver.lib.min.js | 4958f109843696fe5f7d38322e7390f23bae46b0b7c082c4377f0defbc7fa31c |

## 运行边界 / Runtime

[challenge.rs](../challenge.rs) 通过 `include_str!` 嵌入资源，为 QuickJS 设置执行时间、内存和栈限制，不向脚本暴露网络、文件或应用 IPC。PoToken 获取由独立的 [web_po.rs](../web_po.rs) 隐藏 WebView 流程处理。

## 验证 / Verification

从仓库根目录运行合成 player fixture：

```bash
pnpm test:youtube-challenge
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib api::youtube::challenge
```

Node 检查入口是 [test-youtube-challenge.mjs](../../../../../scripts/test-youtube-challenge.mjs)，Rust 检查覆盖实际 QuickJS 执行与 sandbox 限制。合成 fixture 不足以证明当前线上 player、账号会话或 PoToken 流程可用，后者仍需客户端验证。

## 许可 / License

保留文件顶部的全部版权和许可注释：上游 yt-dlp/ejs 使用 Unlicense，捆绑 Meriyah 使用 ISC，Astring 使用 MIT。重新压缩或替换资源时也应保留完整声明。

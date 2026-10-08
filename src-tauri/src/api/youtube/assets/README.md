# YouTube challenge solver

这两个文件原样来自 Android NeriPlayer `3e1abcb704a76a3cd211878c7303d4058866c4d4`
的 `modules/platform/src/main/assets/youtube/`，用于实际 `sig` 和 `n` challenge 解算。

| 文件 | SHA-256 |
| --- | --- |
| yt.solver.core.min.js | 333f68fff3afa5761db121a7af203dd61da8215cc1f58ffb6e4f3df63da224c5 |
| yt.solver.lib.min.js | 4958f109843696fe5f7d38322e7390f23bae46b0b7c082c4377f0defbc7fa31c |

文件顶部保留上游 yt-dlp/ejs 的 Unlicense 声明，以及捆绑 Meriyah 的 ISC
和 Astring 的 MIT 完整许可证与版权说明，不要移除这些注释。

生产运行使用有执行时间、内存和栈限制的嵌入 QuickJS；不暴露网络、文件或应用 IPC。
合成 player fixture 经实际 Rust QuickJS 与 Node 检查；线上 player 和隐藏 WebView
PoToken 的真实运行仍需要客户端验证。

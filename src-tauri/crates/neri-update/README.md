[English](#english) | [中文](#中文)

# neri-update

## 中文

应用内增量更新。不依赖 Tauri 与项目内其它 crate；网络下载在默认开启的 `client` feature 里，发布工具用 `--no-default-features` 构建。

| 模块 | 职责 |
| --- | --- |
| `manifest` | 发布清单模型与结构校验（路径穿越、越界、大小写重名） |
| `signing` | Ed25519 签名与校验，私钥只在发布流水线里 |
| `codec` | SHA-256，完整 zstd 帧，以旧文件为前缀的 zstd 补丁 |
| `layout` | 识别安装形态：Windows NSIS 目录、macOS .app、Linux AppImage；其余给出不支持的原因 |
| `plan` | 比对本地文件哈希，逐文件决定不变 / 打补丁 / 下载完整内容 |
| `stage` | 下载、解码、校验后写入暂存目录，可断点续用 |
| `apply` | 原地替换并在失败时回滚；启动时清理让位文件 |
| `pack` | 发布侧：扫描安装后的文件树，生成 blob、补丁与平台清单 |
| `client` | HTTP Range 下载（服务器不支持 Range 时退回顺序读取） |

### 发布资产

每个平台一份完整内容 blob（`*.update`，每个文件一个 zstd 帧），一份相对上一版本的补丁 blob（`*.from-<旧版本>.patch`），以及所有平台共用的 `neri-update.json` 与签名 `neri-update.json.sig`。客户端按清单里的 offset/length 用 Range 只取变化的文件；本地文件哈希与新版一致的直接跳过，所以即使没有补丁也只下载变化的文件。

### 替换策略

- Windows（NSIS，安装在 `%LOCALAPPDATA%`）：占用中的 exe/dll 先改名为 `*.neri-old` 让位，新文件改名到位；任一步失败按相反顺序还原。
- macOS：在 `.app` 旁拼出完整的新包再整体换名，新包与发布包逐字节一致，ad-hoc 签名保持有效。
- Linux AppImage：同目录写好新文件后 rename 覆盖。
- MSI、deb/rpm、Flatpak、从 DMG 或随机化路径运行时不做原地更新，界面引导到发布页。

### 签名密钥

```bash
cargo run -p neri-update --no-default-features --bin neri-update-tool -- keygen ~/neri-update.key
```

命令把私钥写进指定文件（权限 0600），把公钥打印到标准输出。公钥写进 `src-tauri/update-public-key.txt`；私钥存为仓库 Secret `NERI_UPDATE_SIGNING_KEY`，不要提交进仓库。公钥文件为空时应用内更新关闭，发布流水线跳过增量资产。

```bash
cargo test -p neri-update
```

## English

In-app incremental updates. No dependency on Tauri or other project crates; HTTP downloads live behind the default `client` feature, and the release tool builds with `--no-default-features`.

| Module | Responsibility |
| --- | --- |
| `manifest` | Release manifest model and structural validation (path traversal, out-of-range offsets, case-insensitive duplicates) |
| `signing` | Ed25519 signing and verification; the private key only exists in the release pipeline |
| `codec` | SHA-256, full zstd frames, and zstd patches that use the old file as a prefix |
| `layout` | Detects the install type: Windows NSIS directory, macOS .app, Linux AppImage; otherwise reports why in-place updates are unavailable |
| `plan` | Compares local file hashes and decides per file: unchanged, patch, or full download |
| `stage` | Downloads, decodes and verifies into a staging directory that later runs can reuse |
| `apply` | Swaps files in place with rollback on failure; cleans up set-aside files at startup |
| `pack` | Release side: scans the installed tree and writes blobs, patches and the platform manifest |
| `client` | HTTP Range downloads, falling back to sequential reads when the server ignores Range |

### Release assets

Each platform gets one full-content blob (`*.update`, one zstd frame per file) and one patch blob against the previous release (`*.from-<old version>.patch`). All platforms share `neri-update.json` and its signature `neri-update.json.sig`. Clients fetch only the changed files with Range requests; files whose local hash already matches are skipped, so even without patches only changed files are downloaded.

### Swap strategy

- Windows (NSIS, installed under `%LOCALAPPDATA%`): in-use exe/dll files are renamed to `*.neri-old`, then new files are renamed into place; any failure restores everything in reverse order.
- macOS: a complete new bundle is assembled next to the `.app` and swapped in by rename. It is byte-identical to the released bundle, so the ad-hoc signature stays valid.
- Linux AppImage: the new file is written in the same directory and renamed over the old one.
- MSI, deb/rpm, Flatpak, and apps running from a DMG or a translocated path are not updated in place; the UI points to the releases page.

### Signing key

```bash
cargo run -p neri-update --no-default-features --bin neri-update-tool -- keygen ~/neri-update.key
```

The command writes the private key to the given file (mode 0600) and prints the public key. Put the public key into `src-tauri/update-public-key.txt` and store the private key as the `NERI_UPDATE_SIGNING_KEY` repository secret; never commit it. With an empty public key file, in-app updates are off and the release pipeline skips the incremental assets.

```bash
cargo test -p neri-update
```

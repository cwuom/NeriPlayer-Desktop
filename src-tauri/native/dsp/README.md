# Android DSP 核心 / Android DSP Core

本目录保存 Android DSP 核心、桌面 C ABI 桥接和独立 host 测试。Rust 通过桥接处理交错 float PCM，不构建 JNI 或 Android 音频组件。

## 来源与许可 / Provenance and License

`neri_dsp_*` 核心以 [NeriPlayer Android](https://github.com/cwuom/NeriPlayer/tree/c54884d3600368227f4d6ba86396df5556094199/modules/native/src/main/cpp/dsp) 提交 `c54884d3600368227f4d6ba86396df5556094199` 为基础。低音增强和输出限幅同步 [Android 低音优化](https://github.com/cwuom/NeriPlayer/commit/d61c977cad9c4eae79f78021cf717bb0f018221b) 提交 `d61c977cad9c4eae79f78021cf717bb0f018221b`。

沿用上游根目录 GPL-3.0 许可证，完整文本见 [LICENSE](./LICENSE)。此目录不采用上游 USB 路径的替代授权。更新核心时应保留来源、版权和许可说明，并区分 Android 同步改动与桌面扩展。

## 集成边界 / Integration

| 入口 | 职责 |
| --- | --- |
| [neri_dsp_engine.cpp](./neri_dsp_engine.cpp) | DSP 链、参数下发、重置与处理 |
| [neri_dsp_params.h](./neri_dsp_params.h) | C++ 参数索引和布局 |
| [neri_dsp_bridge.cpp](./neri_dsp_bridge.cpp) | 桌面 C ABI、预建质量模式和统计 |
| [audio/dsp.rs](../../src/audio/dsp.rs) | Rust FFI、原子参数快照和处理状态 |
| [audio/dsp_settings.rs](../../src/audio/dsp_settings.rs) | 设置模型、余量和桌面参数转换 |

参数布局前 104 项与 Android 原始布局逐项一致，末尾追加 2 项桌面扩展，共 106 项。保留音效分区、默认值和原有参数语义；桌面扩展的范围见下文。低音余量计算及两个低音预设对齐 `d61c977` 的 Kotlin 模型。

建流时预建三种质量引擎，音频线程独占引擎，实时参数由 Rust 原子快照传入。质量切换重置预建引擎，回调不调用 configure，也不分配内存。单、双声道走原始 DSP 链，多声道旁路并上报 `unsupported_format`。

## 低音与输出保护 / Bass and Output Protection

正低音提升采用完整搁架和随增益平滑收紧的次声高通。低音、高音及虚拟低音只对新增信号逐样本限量：固定上限 0.87，保持 40 ms，以 250 ms 释放，左右声道联动。负低音在捕获干声前处理，负高音在增强限量后处理，避免衰减被拉回。

增强保护独立于输出限幅开关；输出限幅另有 15 ms 保持。自动余量按正低音搁架增益的 30% 和同一次声高通估算，显示曲线保留完整搁架。例如 +6 dB、100 Hz 低音预留约 1.25 dB，瞬时低频峰值由增强限量处理。

## 桌面扩展 / Desktop Extensions

- 响度补偿：`kLoudnessBassDb` / `kLoudnessTrebleDb` 驱动 120 Hz 低频搁架和 10 kHz 高频搁架，位于低音、高音及虚拟低音的增强限量之后。增益由 Rust 控制端按播放器音量和用户强度计算（音量每降 1 dB 低频补 0.3 dB、高频补 0.1 dB，封顶 10 dB / 4 dB），仍全额计入自动余量。
- 精确图示 EQ：Rust 控制端对 10 段目标增益做正则化最小二乘补偿后再下发，使各中心频率的合成频响等于滑块值；C++ 图示 EQ 结构和 Q 不变。默认关闭，Android 导入的音效保持原叠加听感。
- 参数范围：用户前级允许 -24 dB，内部前级允许 -48 dB，容纳 AutoEQ 衰减和最多 18 dB 自动余量。PEQ 下界扩展至 -30 dB，上界仍为 +24 dB，保留旧 5 段 EQ 扣除整体余量后的极端曲线；10 段图示 EQ、低音和高音的原范围不变。
- 质量切换与禁用：`resetToIdle` 清空预建模式的旧尾音并重新淡入。禁用在当前实例内完成淡出，不因质量模式的默认值改变而跳过过渡。
- x86 兼容：非规格化浮点清零同时覆盖 MSVC 的架构宏。

## 调节连续性 / Parameter Transitions

扬声器响度从压缩比 1 逐渐过渡到原有最大强度，避免零点附近的音量台阶。保护频率沿用原有时间常数平滑变化，并保留滤波历史。压缩器补偿增益使用 25 ms 平滑，三种质量模式保持相同过渡时间。

次声高通启停使用 25 ms 平滑和逐样本交叉淡化。低音从正值调回零或负值时保留尾部滤波历史，稳态零增益精确旁路，负增益保持静态搁架。增强限量按本块实际平滑增益分类，目标穿零不会提前将衰减送入限量；参数单位、范围和最大强度不变。

## 测试与验证 / Testing

[neri_dsp_engine_test.cpp](./tests/neri_dsp_engine_test.cpp) 注册 31 项 host 回归，覆盖中性旁路、音效参数、扬声器响度零点、保护频率拖动、压缩补偿增益和响度补偿。低音用例还覆盖次声抑制、中频泵动、底鼓力度、强段后恢复、谐波失真、正负增益穿零，以及单声道和不同采样率。与五个核心 cpp 使用 C++17 编译运行；Rust FFI 测试额外覆盖桌面参数、分区、谱线、质量切换与唱片效果。

[realtime_allocation_test.cpp](./tests/realtime_allocation_test.cpp) 是独立可执行测试，不链接进应用。它与五个核心 cpp 和 bridge 一起构建，在预建引擎后统计 300 次质量切换、参数下发、重置和处理期间的 C++ heap 分配，要求计数为零。场景包含三种质量模式、低音和高音正负增益、虚拟低音、混响、环绕、扬声器保护与谐波、压缩器和限幅器。

### Windows 实时分配检查

以下为 `cmd` / Visual Studio 开发者命令提示符示例。先建立 `src-tauri/target/dsp-realtime-check` 并进入该目录，再运行；`vcvars64.bat` 路径应替换为本机 Visual Studio 安装路径：

```bat
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cl /nologo /EHsc /std:c++17 /utf-8 /W4 /O2 /MD /I"..\..\native" "..\..\native\dsp\tests\realtime_allocation_test.cpp" "..\..\native\dsp\neri_dsp_engine.cpp" "..\..\native\dsp\neri_dsp_primitives.cpp" "..\..\native\dsp\neri_dsp_filter_stages.cpp" "..\..\native\dsp\neri_dsp_space_stages.cpp" "..\..\native\dsp\neri_dsp_dynamics_stages.cpp" "..\..\native\dsp\neri_dsp_bridge.cpp" /Fe:realtime-allocation-test.exe
realtime-allocation-test.exe
```

进程退出码应为 0，输出应包含 `0 allocations` 和 `valid=1`。这项检查只约束所覆盖的 C++ heap 分配路径，不代替实际播放、音质或 Rust 集成验证。通用开发与验证入口见根目录 [贡献指南](../../../CONTRIBUTING.md)。

# Android DSP 核心来源

`neri_dsp_*` 核心文件以 [NeriPlayer Android](https://github.com/cwuom/NeriPlayer/tree/c54884d3600368227f4d6ba86396df5556094199/modules/native/src/main/cpp/dsp) 提交 `c54884d3600368227f4d6ba86396df5556094199` 为基础，低音增强和输出限幅处理同步 [Android 低音优化](https://github.com/cwuom/NeriPlayer/commit/d61c977cad9c4eae79f78021cf717bb0f018221b) 提交 `d61c977cad9c4eae79f78021cf717bb0f018221b`。遵循上游根目录 GPL-3.0 许可证，完整文本见本目录 `LICENSE`；此目录不采用上游 USB 路径的替代授权。

JNI 和 Android 音频组件没有纳入桌面构建。桌面新增 `neri_dsp_bridge.cpp` 提供 C ABI，复用原引擎处理交错 float PCM；参数布局前 104 项与原始布局逐项一致，末尾追加 2 项桌面扩展（共 106 项）。音效分区、默认值和参数范围保持兼容，低音余量计算及两个低音预设对齐 `d61c977` 的 Kotlin 模型。

正低音提升使用完整搁架和随增益平滑收紧的次声高通。低音、高音及虚拟低音只对新增信号逐样本限量，固定上限 0.87，保持 40 ms 后以 250 ms 释放，左右声道联动；负低音在捕获干声前处理，负高音在增强限量后处理，避免衰减被拉回。该保护独立于输出限幅开关，输出限幅另增加 15 ms 保持。自动余量按正低音搁架增益的 30% 和同一次声高通估算，显示曲线保留完整搁架；例如 +6 dB、100 Hz 低音仅预留约 1.25 dB，瞬时低频峰值由增强限量处理。

桌面音质扩展：

- 响度补偿：`kLoudnessBassDb` / `kLoudnessTrebleDb` 驱动 120 Hz 低频搁架和 10 kHz 高频搁架，位于低音、高音及虚拟低音的增强限量之后。增益由 Rust 控制端按播放器音量和用户强度计算（音量每降 1 dB 低频补 0.3 dB、高频补 0.1 dB，封顶 10 dB / 4 dB），仍全额计入自动余量。
- 精确图示 EQ：Rust 控制端对 10 段目标增益做正则化最小二乘补偿后再下发，使各中心频率的合成频响等于滑块值；C++ 图示 EQ 结构和 Q 不变。默认关闭，Android 导入的音效保持原叠加听感。

桌面端的局部扩展：x86 非规格化浮点清零覆盖 MSVC 的架构宏；用户前级允许 -24 dB，内部前级允许 -48 dB，以容纳 AutoEQ 衰减和最多 18 dB 自动余量；`resetToIdle` 支持清空预建模式的旧尾音并重新淡入。禁用在当前实例内完成淡出，不因质量模式的默认值改变而跳过过渡。

PEQ 衰减下界扩展至 -30 dB，上界仍为 +24 dB，保留旧 5 段 EQ 在整体余量扣除后的极端曲线。10 段图示 EQ、低音和高音的原范围不变。

建流时预建三种质量引擎。音频线程独占引擎，实时参数由 Rust 原子快照传入；质量切换先重置预建引擎，回调不调用 configure，不分配内存。单、双声道支持原始链，多声道明确旁路并上报 unsupported_format。

`tests/neri_dsp_engine_test.cpp` 包含 31 项 host 回归：上游 23 项、桌面扬声器响度零点、保护频率拖动、压缩补偿增益和响度补偿，以及低音衰减配合高音提升、单声道和不同采样率、低音穿零连续性、低音和高音目标穿零时的衰减连续性。低音覆盖次声抑制、中频泵动、底鼓力度、强段后恢复及谐波失真，可与五个核心 cpp 使用 C++17 编译运行。Rust FFI 测试额外覆盖桌面参数、分区、谱线、质量切换与唱片效果。

桌面调节连续性扩展：扬声器响度从压缩比 1 逐渐过渡到原有最大强度，避免零点附近的音量台阶；保护频率使用原有时间常数平滑变化并保留滤波历史；压缩器补偿增益使用 25 ms 平滑，三种质量模式保持相同过渡时间。次声高通启停也使用 25 ms 平滑和逐样本交叉淡化，低音从正值调回零或负值时保留尾部滤波历史，稳态零增益精确旁路、负增益保持静态搁架。增强限量按本块实际平滑增益分类，目标穿零不会提前将衰减送入限量。参数单位、范围和最大强度不变。

`tests/realtime_allocation_test.cpp` 是独立可执行测试，不链接进应用。与五个核心 cpp 和 bridge 使用 C++17 编译运行后，会统计 300 次质量切换、参数下发、重置和处理期间的 C++ heap 分配，并要求计数为零。测试包含三种质量模式、低音和高音正负增益切换、虚拟低音、混响、环绕、扬声器保护与谐波、压缩器与限幅器。

本机验证使用以下实际命令，工作目录为预先建立的 `src-tauri/target/dsp-realtime-check`。其它 Windows 构建机可将 `vcvars64.bat` 路径替换为已安装 Visual Studio 的路径：

```bat
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cl /nologo /EHsc /std:c++17 /utf-8 /W4 /O2 /MD /I"..\..\native" "..\..\native\dsp\tests\realtime_allocation_test.cpp" "..\..\native\dsp\neri_dsp_engine.cpp" "..\..\native\dsp\neri_dsp_primitives.cpp" "..\..\native\dsp\neri_dsp_filter_stages.cpp" "..\..\native\dsp\neri_dsp_space_stages.cpp" "..\..\native\dsp\neri_dsp_dynamics_stages.cpp" "..\..\native\dsp\neri_dsp_bridge.cpp" /Fe:realtime-allocation-test.exe
realtime-allocation-test.exe
```

//! 编译原生 DSP 引擎与 FFmpeg 运行时加载垫片；静态库随本 crate 链接进宿主

use std::env;

fn main() {
    build_ffmpeg_shim();
    build_native_dsp();
}

/// 编译 FFmpeg 运行时加载垫片：只依赖仓库里固定版本的头文件，不需要构建机上装 FFmpeg
fn build_ffmpeg_shim() {
    println!("cargo:rerun-if-changed=native/ffmpeg/neri_ffmpeg.cpp");
    println!("cargo:rerun-if-changed=native/ffmpeg/include");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let include = std::path::Path::new(&manifest_dir).join("native/ffmpeg/include");
    let mut build = cc::Build::new();
    build.cpp(true).file("native/ffmpeg/neri_ffmpeg.cpp");
    // cc 默认开 /W4 或 -Wall -Wextra；FFmpeg 头文件按外部头处理，告警只针对垫片自己的代码
    if build.get_compiler().is_like_msvc() {
        build
            .flag("/std:c++17")
            .flag("/utf-8")
            .flag("/external:W0")
            .flag(format!("/external:I{}", include.display()));
    } else {
        build
            .flag("-std=c++17")
            .flag("-isystem")
            .flag(include.display().to_string());
    }
    build.compile("neri_ffmpeg");
    if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        println!("cargo:rustc-link-lib=dl");
    }
}

fn build_native_dsp() {
    println!("cargo:rerun-if-changed=native/dsp");
    let mut build = cc::Build::new();
    build.cpp(true).include("native");
    for source in [
        "engine",
        "primitives",
        "filter_stages",
        "space_stages",
        "dynamics_stages",
        "bridge",
    ] {
        build.file(format!("native/dsp/neri_dsp_{source}.cpp"));
    }
    build.file("native/dsp/tests/host_test_bridge.cpp");
    if build.get_compiler().is_like_msvc() {
        build.flag("/std:c++17").flag("/utf-8").flag("/EHsc");
    } else {
        build.flag("-std=c++17");
    }
    build.compile("neri_dsp");
}

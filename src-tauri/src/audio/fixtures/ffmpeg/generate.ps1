# 重新生成 FFmpeg 解码测试素材：全部由 lavfi 合成的正弦波编码而成，不含外部音乐
# 用法：pwsh src-tauri/src/audio/fixtures/ffmpeg/generate.ps1 [-Ffmpeg <ffmpeg.exe 路径>]
param(
    [string]$Ffmpeg = $(if ($env:NERI_FFMPEG_DIR) { Join-Path $env:NERI_FFMPEG_DIR "ffmpeg.exe" } else { Join-Path $PSScriptRoot "..\..\..\..\..\.cache\ffmpeg\windows-x86_64\bin\ffmpeg.exe" })
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Ffmpeg)) { throw "ffmpeg not found: $Ffmpeg" }
Set-Location $PSScriptRoot

function Encode([string[]]$Arguments) {
    & $Ffmpeg -y -hide_banner -loglevel error @Arguments
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed: $($Arguments -join ' ')" }
}

$exact = @("-fflags", "+bitexact", "-flags:a", "+bitexact", "-map_metadata", "-1")
$stereo48 = "aevalsrc=exprs=0.4*sin(2*PI*440*t)|0.3*sin(2*PI*660*t):sample_rate=48000"
$stereo44 = "aevalsrc=exprs=0.4*sin(2*PI*440*t)|0.3*sin(2*PI*660*t):sample_rate=44100"

Encode (@("-f", "lavfi", "-i", "${stereo48}:duration=0.5", "-c:a", "pcm_s16le") + $exact + @("pcm-s16-stereo-0.5s.wav"))
Encode (@("-i", "pcm-s16-stereo-0.5s.wav", "-c:a", "flac") + $exact + @("flac-s16-stereo-0.5s.flac"))
Encode (@("-f", "lavfi", "-i", "${stereo48}:duration=1", "-c:a", "libopus", "-b:a", "64k") + $exact + @("opus-stereo-1s.webm"))
Encode (@("-f", "lavfi", "-i", "${stereo44}:duration=1", "-c:a", "libmp3lame", "-b:a", "128k") + $exact + @("mp3-stereo-1s.mp3"))
Encode (@("-f", "lavfi", "-i", "${stereo44}:duration=1", "-c:a", "aac", "-b:a", "128k") + $exact + @("aac-stereo-1s.m4a"))
# 只有中置有信号的 5.1 声床，分片 MP4 与 B 站 DASH 音轨的封装一致
Encode (@("-f", "lavfi", "-i", "aevalsrc=exprs=0|0|0.5*sin(2*PI*1000*t)|0|0|0:channel_layout=5.1:sample_rate=48000:duration=0.5",
          "-c:a", "eac3", "-b:a", "192k", "-movflags", "+frag_keyframe+empty_moov+delay_moov+default_base_moof", "-f", "mp4") + $exact + @("eac3-5.1-center.mp4"))

Get-ChildItem -File | Where-Object { $_.Extension -ne ".ps1" -and $_.Extension -ne ".md" } | Sort-Object Name | ForEach-Object {
    "| {0} | {1} | {2} |" -f $_.Name, $_.Length, (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}

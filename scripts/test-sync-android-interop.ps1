#requires -Version 7.0
<#
.SYNOPSIS
用 Android 原始 Compact 解码器和原始 ProtoBuf 数据类验证 Desktop 导出的固定 V4 样例
.DESCRIPTION
只读取 Android 源码与现有 Gradle 缓存，不安装依赖，不修改 Android 文件，不访问云端
runner 仅复制序列化构造声明并移除 Android 专用方法和接口，字段、默认值和 ProtoNumber 保持原样
此检查不等价于 Android 整仓 Gradle、V4Bridge/provider 或设备互通测试
.EXAMPLE
./scripts/test-sync-android-interop.ps1 -AndroidRoot E:/AndroidProject/NeriPlayer -ExportFixtures
.EXAMPLE
./scripts/test-sync-android-interop.ps1 -AndroidRoot E:/AndroidProject/NeriPlayer -FixtureRoot C:/path/to/exported-fixtures
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AndroidRoot,
    [string]$FixtureRoot,
    [switch]$ExportFixtures,
    [string]$OutputDirectory,
    [string]$GradleCache = (Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1'),
    [string]$PythonPath = 'python',
    [string]$NodePath = 'node',
    [string]$JavaPath = 'java',
    [string]$KotlinVersion = '2.4.10',
    [string]$SerializationVersion = '1.11.0'
)
$ErrorActionPreference = 'Stop'
$AndroidRoot = (Resolve-Path -LiteralPath $AndroidRoot).Path
$interopCache = (Resolve-Path -LiteralPath $GradleCache).Path
$workspace = Split-Path -Parent $PSScriptRoot
if (-not $ExportFixtures -and [string]::IsNullOrWhiteSpace($FixtureRoot)) {
    throw 'Provide -FixtureRoot or -ExportFixtures'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) ('neriplayer-android-interop-' + [guid]::NewGuid().ToString('N'))
}
$interopOutput = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $interopOutput) { throw 'OutputDirectory must be new to preserve existing data' }
[IO.Directory]::CreateDirectory($interopOutput) | Out-Null
Write-Host "Audit artifacts retained in $interopOutput"
$interopIgnore = Join-Path $interopOutput '.gitignore'
[IO.File]::WriteAllText($interopIgnore, "*`n", [Text.UTF8Encoding]::new($false))
$fixtureOutput = Join-Path $interopOutput 'fixtures'
if ($ExportFixtures) {
    $previousFixtureEnv = $env:NERI_CODEC_FIXTURE_DIR
    try {
        $env:NERI_CODEC_FIXTURE_DIR = $fixtureOutput
        & cargo test --manifest-path (Join-Path $workspace 'src-tauri/Cargo.toml') --locked --lib sync::archive::tests::export_android_interop_fixtures_when_requested -- --exact --nocapture
        if ($LASTEXITCODE -ne 0) { throw 'Desktop fixture export failed' }
    } finally { $env:NERI_CODEC_FIXTURE_DIR = $previousFixtureEnv }
} else {
    $FixtureRoot = (Resolve-Path -LiteralPath $FixtureRoot).Path
    [IO.Directory]::CreateDirectory($fixtureOutput) | Out-Null
    foreach ($name in @('desktop-empty', 'desktop-nonempty')) {
        Copy-Item -LiteralPath (Join-Path $FixtureRoot $name) -Destination $fixtureOutput -Recurse
    }
}
$androidSourceRoot = Join-Path $AndroidRoot 'modules'
$archiveSource = Join-Path $androidSourceRoot 'sync/src/main/java/moe/ouom/neriplayer/data/sync/archive'
$modelSource = Join-Path $androidSourceRoot 'model/src/main/java/moe/ouom/neriplayer'
$sourceFiles = @(
    (Join-Path $modelSource 'data/model/sync/SyncDataModels.kt'),
    (Join-Path $modelSource 'data/model/sync/SyncBiliVideoSkipModels.kt'),
    (Join-Path $modelSource 'data/sync/model/SyncCausalToken.kt'),
    (Join-Path $modelSource 'data/model/playlist/LocalPlaylist.kt'),
    (Join-Path $archiveSource 'SyncArchiveModels.kt'),
    (Join-Path $archiveSource 'SyncLegacyLyricArchive.kt'),
    (Join-Path $archiveSource 'v4/SyncArchiveV4Format.kt'),
    (Join-Path $archiveSource 'SyncArchiveStaging.kt')
)
$sourceFiles += Get-ChildItem -LiteralPath (Join-Path $archiveSource 'compact') -Filter '*.kt' | ForEach-Object FullName
$sourceFiles | ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 } |
    Select-Object Path, Hash | Export-Csv -LiteralPath (Join-Path $interopOutput 'android-source-sha256.csv') -Encoding utf8 -NoTypeInformation
$androidHead = & git -C $AndroidRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Android reference commit' }
[IO.File]::WriteAllText((Join-Path $interopOutput 'versions.txt'), "Android=$androidHead`nKotlin=$KotlinVersion`nSerialization=$SerializationVersion`n", [Text.UTF8Encoding]::new($false))
$manifestPaths = @('desktop-empty', 'desktop-nonempty') | ForEach-Object { Join-Path $fixtureOutput "$_/manifest.pb" }
$manifestPaths | ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 } |
    Select-Object Path, Hash | Export-Csv -LiteralPath (Join-Path $interopOutput 'desktop-manifest-sha256.csv') -Encoding utf8 -NoTypeInformation
$extractSource = @'
from pathlib import Path
import re

import sys
android = Path(sys.argv[1])
base = android / 'modules/model/src/main/java/moe/ouom/neriplayer'
archive = android / 'modules/sync/src/main/java/moe/ouom/neriplayer/data/sync/archive'
paths = [base/'data/model/sync/SyncDataModels.kt', base/'data/model/sync/SyncBiliVideoSkipModels.kt', base/'data/sync/model/SyncCausalToken.kt', archive/'SyncArchiveModels.kt', archive/'SyncLegacyLyricArchive.kt', archive/'v4/SyncArchiveV4Format.kt']
declarations = []
for path in paths:
    source = path.read_text(encoding='utf-8')
    for match in re.finditer(r'@Serializable\s+(?:internal\s+)?data class \w+\(', source):
        start = match.end() - 1
        depth, quoted, escaped = 0, False, False
        for index in range(start, len(source)):
            char = source[index]
            if quoted:
                if escaped: escaped = False
                elif char == '\\': escaped = True
                elif char == '"': quoted = False
            elif char == '"': quoted = True
            elif char == '(': depth += 1
            elif char == ')':
                depth -= 1
                if depth == 0:
                    declarations.append(source[match.start():index+1])
                    break
    if path.name == 'SyncDataModels.kt':
        start = source.index('@Serializable\nenum class SyncAction')
        end = source.index('}', start) + 1
        declarations.append(source[start:end])
output = Path(__file__).with_name('Models.kt')
constants = []
for name, path in [('LEGACY_SONG_ORDER_VERSION', base/'data/model/playlist/LocalPlaylist.kt'), ('LEGACY_SYNC_METADATA_VERSION', base/'data/model/sync/SyncDataModels.kt')]:
    value = re.search(r'const val ' + name + r'\s*=\s*(\d+)\b', path.read_text(encoding='utf-8'))
    if value is None: raise RuntimeError('Android constant not found: ' + name)
    constants.append('const val ' + name + ' = ' + value.group(1))
output.write_text('@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)\npackage interop\nimport kotlinx.serialization.Serializable\nimport kotlinx.serialization.protobuf.ProtoNumber\n' + '\n'.join(constants) + '\n\n' + '\n\n'.join(declarations), encoding='utf-8')
print('Copied', len(declarations), 'actual Android serializable declarations without changing constructor fields/defaults')
'@
$probeSource = @'
@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)
package interop
import kotlinx.serialization.decodeFromByteArray
import kotlinx.serialization.encodeToByteArray
import kotlinx.serialization.protobuf.ProtoBuf
import java.io.File
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.security.MessageDigest
import moe.ouom.neriplayer.data.sync.archive.compact.SyncArchiveCompactRecords

fun hash(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
private fun stream(directory: File, value: SyncArchiveV4Stream): ByteArray {
    fun read(ref: SyncArchiveV4Ref?): ByteArray {
        if (ref == null) return byteArrayOf()
        val bytes = File(directory, "neriplayer-sync-v4-" + ref.hash + ".zst.raw").readBytes()
        require(bytes.size == ref.rawBytes && hash(bytes) == ref.rawHash)
        if (!ref.index) return bytes
        return ProtoBuf.decodeFromByteArray<SyncArchiveV4Index>(bytes).children.flatMap { read(it).asList() }.toByteArray()
    }
    val bytes = read(value.root)
    require(bytes.size.toLong() == value.rawBytes)
    return bytes
}

fun records(bytes: ByteArray, expected: Long): List<Pair<Int, Any>> {
    val result = mutableListOf<Pair<Int, Any>>()
    val input = DataInputStream(ByteArrayInputStream(bytes))
    while (input.available() > 0) {
        val kind = input.readUnsignedByte()
        val size = input.readInt()
        require(size >= 0 && size <= input.available())
        val payload = ByteArray(size).also(input::readFully)
        val record: Any = when (kind) {
            1 -> ProtoBuf.decodeFromByteArray<SyncPlaylist>(payload)
            2, 4, 15 -> ProtoBuf.decodeFromByteArray<SyncSong>(payload)
            3 -> ProtoBuf.decodeFromByteArray<SyncFavoritePlaylist>(payload)
            5 -> ProtoBuf.decodeFromByteArray<SyncRecentPlay>(payload)
            6 -> ProtoBuf.decodeFromByteArray<SyncLogEntry>(payload)
            7 -> ProtoBuf.decodeFromByteArray<SyncRecentPlayDeletion>(payload)
            8 -> ProtoBuf.decodeFromByteArray<SyncTrackStat>(payload)
            9 -> ProtoBuf.decodeFromByteArray<SyncPlaybackStatBucket>(payload)
            10 -> ProtoBuf.decodeFromByteArray<SyncPlaylistSongDeletion>(payload)
            11 -> ProtoBuf.decodeFromByteArray<SyncPlaylistUsageStat>(payload)
            12 -> ProtoBuf.decodeFromByteArray<SyncLocalPlaylistPlaybackStat>(payload)
            13 -> ProtoBuf.decodeFromByteArray<SyncLocalPlaylistPlaybackBucket>(payload)
            14 -> ProtoBuf.decodeFromByteArray<SyncBiliVideoSkipRule>(payload)
            16 -> ProtoBuf.decodeFromByteArray<SyncPlaylistUsageDeletion>(payload)
            else -> error("Unknown record kind")
        }
        result.add(kind to record)
    }
    require(result.size.toLong() == expected)
    return result
}

fun main(args: Array<String>) {
    val empty = SyncArchiveManifest(3, SyncData(lastModified = 0), null, 0, 0, 0)
    val encoded = ProtoBuf.encodeToByteArray(empty)
    println("ANDROID_EMPTY_MANIFEST_PROTO=" + encoded.joinToString("") { "%02x".format(it) })
    val stream = SyncArchiveV4Stream(null, 0, 0)
    println("ANDROID_EMPTY_STREAM_PROTO=" + ProtoBuf.encodeToByteArray(stream).joinToString("") { "%02x".format(it) })
    for ((name, bytes) in listOf("omit-zero-original" to byteArrayOf(18, 0), "explicit-zero-original" to byteArrayOf(18, 0, 32, 0, 40, 0, 48, 0))) {
        try {
            ProtoBuf.decodeFromByteArray<SyncArchiveManifest>(bytes)
            require(name == "explicit-zero-original")
            println(name + ": accepted")
        } catch (error: Throwable) {
            require(name == "omit-zero-original" && error.javaClass.simpleName == "MissingFieldException")
            println(name + ": rejected MissingFieldException as required")
        }
    }
    for (path in args) {
        try {
            val manifest = ProtoBuf.decodeFromByteArray<SyncArchiveV4Manifest>(File(path).readBytes())
            println("DESKTOP_PROTO_ACCEPTED " + path + " records=" + manifest.original.recordCount + " bytes=" + manifest.original.rawDataBytes)
            val directory = File(path).parentFile
            val main = stream(directory, manifest.main)
            val legacy = stream(directory, manifest.legacy)
            val pool = stream(directory, manifest.pool)
            val mainOutput = ByteArrayOutputStream()
            val legacyOutput = ByteArrayOutputStream()
            SyncArchiveCompactRecords.unpack(main.inputStream(), legacy.inputStream(), pool.inputStream(), mainOutput, legacyOutput,
                manifest.original.rawDataBytes, manifest.original.legacyLyrics?.rawDataBytes ?: 0L, directory) {}
            require(hash(mainOutput.toByteArray()) == manifest.mainRawHash)
            require(hash(legacyOutput.toByteArray()) == manifest.legacyRawHash)
            val decoded = records(mainOutput.toByteArray(), manifest.original.recordCount)
            val legacyRows = records(legacyOutput.toByteArray(), manifest.original.legacyLyrics?.recordCount ?: 0L)
            println("ANDROID_PRODUCTION_COMPACT_ACCEPTED main=" + decoded.size + " legacy=" + legacyRows.size)
            if (decoded.isEmpty()) {
                require(legacyRows.isEmpty() && manifest.original.recordCount == 0L && manifest.original.rawDataBytes == 0L)
                require(ProtoBuf.decodeFromByteArray<SyncArchiveV4Stream>(byteArrayOf(16, 0, 24, 0)).root == null)
                println("EMPTY_NULL_AND_REQUIRED_ZERO_ASSERTIONS_PASSED")
            } else {
                val playlist = decoded.single { it.first == 1 }.second as SyncPlaylist
                val song = decoded.single { it.first == 2 }.second as SyncSong
                val override = decoded.single { it.first == 15 }.second as SyncSong
                val legacySong = legacyRows.single { it.first == 15 }.second as SyncSong
                require(playlist.id == 123L && playlist.name.toCharArray().map { it.code } == listOf(0x4e92, 0x901a))
                require(song.id == 456L && override.id == 456L && legacySong.id == 456L)
                val expectedLyric = "[00:01.20]" + String(charArrayOf(0x6b4c.toChar(), 0x8bcd.toChar())) + "\n"
                require(song.matchedLyric == expectedLyric && override.matchedLyric == expectedLyric && legacySong.matchedLyric == expectedLyric)
                require(song.matchedTranslatedLyric == null && song.originalLyric == null && song.legacyAddedAt == null)
                require(song.lyricSyncRevision == 1L && song.lyricSyncEdited == true)
                require(override.lyricSyncRevision == 1L && override.lyricSyncEdited == true)
                require(legacySong.lyricSyncRevision == 0L && legacySong.lyricSyncEdited == null)
                println("IDS_CHINESE_NEWLINE_NULL_REVISION_AND_LEGACY_ASSERTIONS_PASSED")
            }
            require(decoded.isEmpty() || (decoded.single { it.first == 15 }.second as SyncSong).name.isEmpty())
        } catch (error: Throwable) { println("DESKTOP_PROTO_REJECTED " + path + " " + error.javaClass.simpleName + ": " + error.message); throw error }
    }
}
'@
$decompressSource = @'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { zstdDecompressSync } from 'node:zlib';
import { createHash } from 'node:crypto';
const hash = data => createHash('sha256').update(data).digest('hex');
for (const directory of process.argv.slice(2)) {
  const envelope = readFileSync(resolve(directory, 'neriplayer-sync-v3.manifest'));
  if (envelope.subarray(0, 8).toString() !== 'NPSYNC04' || hash(envelope.subarray(44)) !== envelope.subarray(12, 44).toString('hex')) throw Error('Manifest envelope checksum');
  const protobuf = zstdDecompressSync(envelope.subarray(44), { maxOutputLength: 1024 * 1024 });
  if (!protobuf.equals(readFileSync(resolve(directory, 'manifest.pb'))) || protobuf.length !== envelope.readUInt32BE(8)) throw Error('Manifest protobuf mismatch');
  for (const file of readdirSync(directory).filter(name => name.endsWith('.zst'))) {
    const compressed = readFileSync(resolve(directory, file));
    if (hash(compressed) !== file.slice('neriplayer-sync-v4-'.length, -4)) throw Error('Compressed object hash mismatch');
    const bytes = zstdDecompressSync(compressed, { maxOutputLength: 4 * 1024 * 1024 });
    writeFileSync(resolve(directory, file + '.raw'), bytes);
  }
}
'@

foreach ($entry in @(@('extract.py', $extractSource), @('Probe.kt', $probeSource), @('decompress.mjs', $decompressSource))) {
    [IO.File]::WriteAllText((Join-Path $interopOutput $entry[0]), $entry[1], [Text.UTF8Encoding]::new($false))
}
& $PythonPath (Join-Path $interopOutput 'extract.py') $AndroidRoot
if ($LASTEXITCODE -ne 0) { throw 'Actual Android model extraction failed' }
$fixturePaths = @('desktop-empty', 'desktop-nonempty') | ForEach-Object { Join-Path $fixtureOutput $_ }
& $NodePath (Join-Path $interopOutput 'decompress.mjs') @fixturePaths
if ($LASTEXITCODE -ne 0) { throw 'Desktop envelope/object validation failed' }
function Artifact([string]$group, [string]$name, [string]$version) {
    $artifactPath = Join-Path $interopCache "$group/$name/$version"
    $jar = Get-ChildItem -LiteralPath $artifactPath -Filter "$name-$version.jar" -Recurse | Select-Object -First 1
    if (-not $jar) { throw "Missing cached $group/$name/$version" }
    return $jar.FullName
}
$stdlib = Artifact 'org.jetbrains.kotlin' 'kotlin-stdlib' $KotlinVersion
$compiler = Artifact 'org.jetbrains.kotlin' 'kotlin-compiler-embeddable' $KotlinVersion
$script = Artifact 'org.jetbrains.kotlin' 'kotlin-script-runtime' $KotlinVersion
$reflect = Artifact 'org.jetbrains.kotlin' 'kotlin-reflect' '1.6.10'
$buildTools = Artifact 'org.jetbrains.kotlin' 'kotlin-build-tools-api' $KotlinVersion
$daemon = Artifact 'org.jetbrains.kotlin' 'kotlin-daemon-embeddable' $KotlinVersion
$coroutines = Artifact 'org.jetbrains.kotlinx' 'kotlinx-coroutines-core-jvm' '1.8.0'
$annotations = Artifact 'org.jetbrains' 'annotations' '23.0.0'
$serialization = Artifact 'org.jetbrains.kotlinx' 'kotlinx-serialization-core-jvm' $SerializationVersion
$protobuf = Artifact 'org.jetbrains.kotlinx' 'kotlinx-serialization-protobuf-jvm' $SerializationVersion
$plugin = Artifact 'org.jetbrains.kotlin' 'kotlin-serialization-compiler-plugin-embeddable' $KotlinVersion
$compilerClasspath = @($compiler, $stdlib, $script, $reflect, $buildTools, $daemon, $coroutines, $annotations) -join ';'
$runtimeClasspath = @($stdlib, $serialization, $protobuf, $annotations) -join ';'
$output = Join-Path $interopOutput 'probe.jar'
$androidArchivePath = Join-Path $AndroidRoot 'modules/sync/src/main/java/moe/ouom/neriplayer/data/sync/archive'
$actualCompactSources = @(Get-ChildItem -LiteralPath (Join-Path $androidArchivePath 'compact') -Filter '*.kt' | ForEach-Object FullName)
$actualCompactSources += Join-Path $androidArchivePath 'SyncArchiveStaging.kt'
& $JavaPath -cp $compilerClasspath org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath $runtimeClasspath "-Xplugin=$plugin" -d $output (Join-Path $interopOutput 'Models.kt') (Join-Path $interopOutput 'Probe.kt') @actualCompactSources
if ($LASTEXITCODE -ne 0) { throw 'Kotlin compile failed' }
& $JavaPath -cp ($output + ';' + $runtimeClasspath) interop.ProbeKt @manifestPaths
if ($LASTEXITCODE -ne 0) { throw 'Kotlin decoder failed' }

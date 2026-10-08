use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use rquickjs::{Context, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::api::transport::FallbackHttp;
use crate::error::{AppError, AppResult};

const MAX_SCRIPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREPROCESSED_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHALLENGES: usize = 64;
// 每个 player 版本只用 meriyah 完整解析一次，之后只运行预处理出的 n/sig 函数
const PREPROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const PREPROCESS_MEMORY_BYTES: usize = 1024 * 1024 * 1024;
const SOLVE_TIMEOUT: Duration = Duration::from_secs(8);
const SOLVE_MEMORY_BYTES: usize = 128 * 1024 * 1024;
// 解析器递归深度随 player 嵌套增长，QuickJS 每层 JS 调用都消耗原生栈；
// 2026-10 的 player 实测需要约 8MB，这里留出数倍余量
const SOLVER_THREAD_STACK_BYTES: usize = 64 * 1024 * 1024;
const SOLVER_JS_STACK_BYTES: usize = SOLVER_THREAD_STACK_BYTES - 16 * 1024 * 1024;
const SCRIPT_TTL: Duration = Duration::from_secs(12 * 60 * 60);
// player 地址自带版本哈希，同一地址的预处理结果不会过期，只按时间兜底清理
const SNAPSHOT_TTL_MS: u64 = 14 * 24 * 60 * 60 * 1000;
const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_LIMIT: usize = 3;
const LIB: &str = include_str!("assets/yt.solver.lib.min.js");
const CORE: &str = include_str!("assets/yt.solver.core.min.js");

struct CachedScript {
    url: String,
    script: Arc<str>,
    fetched_at: Instant,
}

#[derive(Clone)]
struct PlayerRecord {
    url: String,
    signature_timestamp: Option<u64>,
    preprocessed: Option<Arc<str>>,
}

#[derive(Serialize, Deserialize)]
struct PlayerSnapshot {
    version: u32,
    solver: String,
    url: String,
    saved_at_ms: u64,
    signature_timestamp: Option<u64>,
    preprocessed: Option<String>,
}

static SCRIPT_CACHE: LazyLock<Mutex<VecDeque<CachedScript>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
static SCRIPT_FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PLAYERS: LazyLock<Mutex<VecDeque<PlayerRecord>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
// 完整解析占用大量 CPU 和内存，同一时间只跑一份，其余请求等它写入缓存
static PREPROCESS_GATE: Mutex<()> = Mutex::new(());
static WARMING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static SOLVER_FINGERPRINT: LazyLock<String> = LazyLock::new(|| {
    let mut hash = Sha256::new();
    hash.update(LIB.as_bytes());
    hash.update([0]);
    hash.update(CORE.as_bytes());
    hex::encode(&hash.finalize()[..8])
});

#[derive(Default)]
pub(super) struct ChallengeSolutions {
    pub(super) signatures: HashMap<String, String>,
    pub(super) throttling: HashMap<String, String>,
}

pub(super) fn trusted_player_script_url(raw: &str) -> Option<String> {
    let url = url::Url::parse("https://www.youtube.com/")
        .ok()?
        .join(raw)
        .ok()?;
    let host = url.host_str()?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !matches!(host, "www.youtube.com" | "music.youtube.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with("/s/player/")
        || !url.path().ends_with(".js")
    {
        return None;
    }
    Some(url.to_string())
}

fn cached_script(url: &str) -> Option<Arc<str>> {
    let cache = SCRIPT_CACHE.lock().ok()?;
    cache
        .iter()
        .find(|entry| entry.url == url && entry.fetched_at.elapsed() < SCRIPT_TTL)
        .map(|entry| entry.script.clone())
}

async fn player_script(http: &FallbackHttp, url: &str) -> AppResult<Arc<str>> {
    if let Some(script) = cached_script(url) {
        return Ok(script);
    }
    let http = http.clone();
    let url = url.to_owned();
    // 下载放进独立任务：调用方限时放弃后仍会写入缓存，供预热和下一首复用
    tokio::spawn(async move {
        // 播放与预热可能同时需要同一份脚本，只下载一次
        let _fetch = SCRIPT_FETCH.lock().await;
        if let Some(script) = cached_script(&url) {
            return Ok(script);
        }
        download_player_script(&http, &url).await
    })
    .await
    .map_err(|_| AppError::Api("YouTube player script download cancelled".into()))?
}

async fn download_player_script(http: &FallbackHttp, url: &str) -> AppResult<Arc<str>> {
    let started = Instant::now();
    let mut response = http
        .send(|client| client.get(url))
        .await?
        .error_for_status()?;
    if trusted_player_script_url(response.url().as_str()).is_none() {
        return Err(AppError::Api(
            "YouTube player script redirected outside trusted hosts".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_SCRIPT_BYTES {
            return Err(AppError::Api(
                "YouTube player script exceeds size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let script: Arc<str> = String::from_utf8(bytes)
        .map_err(|_| AppError::Api("YouTube player script is not UTF-8".into()))?
        .into();
    log::info!(
        target: "youtube-playback",
        "player script fetched bytes={}, elapsed_ms={}",
        script.len(),
        started.elapsed().as_millis(),
    );
    if let Ok(mut cache) = SCRIPT_CACHE.lock() {
        cache.retain(|entry| entry.url != url);
        cache.push_back(CachedScript {
            url: url.into(),
            script: script.clone(),
            fetched_at: Instant::now(),
        });
        while cache.len() > 2 {
            cache.pop_front();
        }
    }
    Ok(script)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn snapshot_directory() -> Option<PathBuf> {
    dirs_next::cache_dir().map(|root| root.join("NeriPlayer").join("youtube-playback"))
}

fn snapshot_path(directory: &Path, url: &str) -> PathBuf {
    let digest = hex::encode(Sha256::digest(url.as_bytes()));
    directory.join(format!("player-{}.json", &digest[..16]))
}

fn load_snapshot(path: &Path, url: &str, now: u64) -> Option<PlayerRecord> {
    if std::fs::metadata(path).ok()?.len() > (MAX_PREPROCESSED_BYTES + 64 * 1024) as u64 {
        return None;
    }
    let snapshot: PlayerSnapshot = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let age = now.checked_sub(snapshot.saved_at_ms)?;
    if snapshot.version != SNAPSHOT_VERSION
        || snapshot.solver != *SOLVER_FINGERPRINT
        || snapshot.url != url
        || age >= SNAPSHOT_TTL_MS
    {
        return None;
    }
    Some(PlayerRecord {
        url: snapshot.url,
        signature_timestamp: snapshot.signature_timestamp,
        preprocessed: snapshot
            .preprocessed
            .filter(|code| !code.is_empty() && code.len() <= MAX_PREPROCESSED_BYTES)
            .map(Arc::from),
    })
}

fn save_snapshot(directory: &Path, record: &PlayerRecord, now: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let bytes = serde_json::to_vec(&PlayerSnapshot {
        version: SNAPSHOT_VERSION,
        solver: SOLVER_FINGERPRINT.clone(),
        url: record.url.clone(),
        saved_at_ms: now,
        signature_timestamp: record.signature_timestamp,
        preprocessed: record.preprocessed.as_deref().map(str::to_owned),
    })?;
    let path = snapshot_path(directory, &record.url);
    let temporary = directory.join(format!(".player-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return result;
    }
    let mut others = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("player-") && name.ends_with(".json") && entry.path() != path
        })
        .map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, entry.path())
        })
        .collect::<Vec<_>>();
    others.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, stale) in others.into_iter().skip(SNAPSHOT_LIMIT.saturating_sub(1)) {
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}

fn cached_record(url: &str) -> Option<PlayerRecord> {
    if let Some(record) = PLAYERS
        .lock()
        .ok()
        .and_then(|players| players.iter().find(|record| record.url == url).cloned())
    {
        return Some(record);
    }
    let record = load_snapshot(&snapshot_path(&snapshot_directory()?, url), url, now_ms())?;
    Some(remember(record))
}

/// 合并进内存缓存并返回合并后的记录，已知字段不会被空值覆盖
fn remember(update: PlayerRecord) -> PlayerRecord {
    let Ok(mut players) = PLAYERS.lock() else {
        return update;
    };
    let merged = match players.iter().position(|record| record.url == update.url) {
        Some(index) => {
            let previous = players.remove(index).expect("index from position");
            PlayerRecord {
                url: update.url,
                signature_timestamp: update.signature_timestamp.or(previous.signature_timestamp),
                preprocessed: update.preprocessed.or(previous.preprocessed),
            }
        }
        None => update,
    };
    players.push_back(merged.clone());
    while players.len() > 2 {
        players.pop_front();
    }
    merged
}

fn forget_preprocessed(url: &str) {
    if let Ok(mut players) = PLAYERS.lock() {
        for record in players.iter_mut().filter(|record| record.url == url) {
            record.preprocessed = None;
        }
    }
    if let Some(directory) = snapshot_directory() {
        let _ = std::fs::remove_file(snapshot_path(&directory, url));
    }
}

fn persist(record: &PlayerRecord) {
    if let Some(directory) = snapshot_directory() {
        if let Err(error) = save_snapshot(&directory, record, now_ms()) {
            log::warn!(target: "youtube-playback", "player snapshot not saved: {error}");
        }
    }
}

/// 签名时间戳与预处理结果都已缓存时，这个 player 版本可以立即求解
pub(super) fn player_ready(url: &str) -> bool {
    trusted_player_script_url(url)
        .and_then(|url| cached_record(&url))
        .is_some_and(|record| record.preprocessed.is_some())
}

pub(super) fn cached_signature_timestamp(url: &str) -> Option<u64> {
    cached_record(&trusted_player_script_url(url)?)?.signature_timestamp
}

/// player 请求需要签名时间戳；命中缓存时不再为它下载整份脚本
pub(super) async fn signature_timestamp(http: &FallbackHttp, url: &str) -> Option<u64> {
    let url = trusted_player_script_url(url)?;
    if let Some(timestamp) = cached_record(&url).and_then(|record| record.signature_timestamp) {
        return Some(timestamp);
    }
    let script = player_script(http, &url).await.ok()?;
    let timestamp = super::bootstrap::timestamp_from_script(&script)?;
    let record = remember(PlayerRecord {
        url,
        signature_timestamp: Some(timestamp),
        preprocessed: None,
    });
    tokio::task::spawn_blocking(move || persist(&record));
    Some(timestamp)
}

/// 首播前在后台完成整份 player 的解析，播放时只运行预处理结果
pub(super) fn warm(http: FallbackHttp, url: String) {
    let Some(url) = trusted_player_script_url(&url) else {
        return;
    };
    if cached_record(&url).is_some_and(|record| record.preprocessed.is_some())
        || !WARMING
            .lock()
            .map(|mut warming| warming.insert(url.clone()))
            .unwrap_or(false)
    {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(super::BACKGROUND_WARMUP_DELAY).await;
        let started = Instant::now();
        let result = match player_script(&http, &url).await {
            Ok(script) => {
                let target = url.clone();
                on_solver_thread(move || preprocess_and_solve(&target, &script, &[])).await
            }
            Err(error) => Err(error),
        };
        if let Ok(mut warming) = WARMING.lock() {
            warming.remove(&url);
        }
        log::info!(
            target: "youtube-playback",
            "player warmed ok={}, elapsed_ms={}",
            result.is_ok(),
            started.elapsed().as_millis(),
        );
    });
}

fn spawn_solver_thread<F, T>(stack_bytes: usize, job: F) -> AppResult<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name("youtube-ejs".into())
        .stack_size(stack_bytes)
        .spawn(job)
        .map_err(|_| AppError::Api("YouTube JS solver thread unavailable".into()))
}

/// 求解放在独立大栈线程；调用方取消时线程仍会跑完并写入缓存
async fn on_solver_thread<T: Send + 'static>(
    job: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    spawn_solver_thread(SOLVER_THREAD_STACK_BYTES, move || {
        let _ = sender.send(job());
    })?;
    receiver
        .await
        .map_err(|_| AppError::Api("YouTube JS challenge task cancelled".into()))?
}

fn evaluate(
    script: &str,
    budget: Duration,
    memory_bytes: usize,
    stack_bytes: usize,
) -> AppResult<String> {
    let runtime =
        Runtime::new().map_err(|_| AppError::Api("YouTube JS runtime unavailable".into()))?;
    runtime.set_memory_limit(memory_bytes);
    runtime.set_max_stack_size(stack_bytes);
    let deadline = Instant::now() + budget;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime)
        .map_err(|_| AppError::Api("YouTube JS context unavailable".into()))?;
    // 不暴露文件、网络或 IPC 能力，远端脚本只处理 challenge 字符串
    context.with(|ctx| {
        ctx.eval::<String, _>(script).map_err(|error| {
            let detail = if matches!(error, rquickjs::Error::Exception) {
                ctx.catch()
                    .get::<rquickjs::convert::Coerced<String>>()
                    .map(|value| value.0)
                    .unwrap_or_else(|_| "uncaught exception".into())
            } else {
                error.to_string()
            };
            let detail: String = detail.chars().take(240).collect();
            AppError::Api(format!("YouTube JS challenge evaluation failed: {detail}"))
        })
    })
}

fn challenge_requests(signatures: &[String], throttling: &[String]) -> Vec<Value> {
    let mut requests = Vec::new();
    if !signatures.is_empty() {
        requests.push(json!({"type": "sig", "challenges": signatures}));
    }
    if !throttling.is_empty() {
        requests.push(json!({"type": "n", "challenges": throttling}));
    }
    requests
}

fn parse_solutions(requests: &[Value], output: &Value) -> AppResult<ChallengeSolutions> {
    let responses = output["responses"]
        .as_array()
        .ok_or_else(|| AppError::Api("YouTube JS challenge has no result".into()))?;
    let mut solutions = ChallengeSolutions::default();
    for (request, response) in requests.iter().zip(responses) {
        let Some(data) = response["data"]
            .as_object()
            .filter(|_| response["type"] == "result")
        else {
            continue;
        };
        let destination = if request["type"] == "sig" {
            &mut solutions.signatures
        } else {
            &mut solutions.throttling
        };
        for (challenge, solution) in data {
            if let Some(solution) = solution.as_str().filter(|value| {
                !value.is_empty() && value.len() <= 8192 && !value.starts_with("enhanced_except_")
            }) {
                destination.insert(challenge.clone(), solution.into());
            }
        }
    }
    Ok(solutions)
}

fn parse_output(output: &str) -> AppResult<Value> {
    serde_json::from_str(output)
        .map_err(|_| AppError::Api("YouTube JS challenge returned invalid JSON".into()))
}

/// 完整解析 player，同时取出可复用的预处理代码
fn solve_player(
    player: &str,
    requests: &[Value],
    stack_bytes: usize,
) -> AppResult<(ChallengeSolutions, Option<String>)> {
    if player.len() > MAX_SCRIPT_BYTES {
        return Err(AppError::Api(
            "YouTube JS challenge input exceeds limits".into(),
        ));
    }
    let input = json!({
        "type": "player",
        "player": player,
        "requests": requests,
        "output_preprocessed": true,
    });
    let source =
        format!("{LIB}\nObject.assign(globalThis, lib);\n{CORE}\nJSON.stringify(jsc({input}));");
    let output = parse_output(&evaluate(
        &source,
        PREPROCESS_TIMEOUT,
        PREPROCESS_MEMORY_BYTES,
        stack_bytes,
    )?)?;
    let preprocessed = output["preprocessed_player"]
        .as_str()
        .filter(|code| !code.is_empty() && code.len() <= MAX_PREPROCESSED_BYTES)
        .map(str::to_owned);
    Ok((parse_solutions(requests, &output)?, preprocessed))
}

/// 与 EJS core 的 preprocessed 分支等价，省去解析器与 player 原文
fn solve_preprocessed(preprocessed: &str, requests: &[Value]) -> AppResult<ChallengeSolutions> {
    let code = serde_json::to_string(preprocessed)
        .map_err(|_| AppError::Api("YouTube JS challenge input is invalid".into()))?;
    let requests_literal = Value::Array(requests.to_vec());
    let source = format!(
        r#"const _result={{n:null,sig:null}};Function("_result",{code})(_result);
JSON.stringify({{type:"result",responses:{requests_literal}.map(request=>{{
  const solve=_result[request.type];
  if(typeof solve!=="function")return{{type:"error",error:`Failed to extract ${{request.type}} function`}};
  try{{return{{type:"result",data:Object.fromEntries(request.challenges.map(value=>[value,solve(value)]))}}}}
  catch(error){{return{{type:"error",error:String(error)}}}}
}})}});"#
    );
    let output = parse_output(&evaluate(
        &source,
        SOLVE_TIMEOUT,
        SOLVE_MEMORY_BYTES,
        SOLVER_JS_STACK_BYTES,
    )?)?;
    parse_solutions(requests, &output)
}

fn preprocess_and_solve(
    url: &str,
    player: &str,
    requests: &[Value],
) -> AppResult<ChallengeSolutions> {
    let _gate = PREPROCESS_GATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(preprocessed) = cached_record(url).and_then(|record| record.preprocessed) {
        match solve_preprocessed(&preprocessed, requests) {
            Ok(solutions) => return Ok(solutions),
            Err(_) => forget_preprocessed(url),
        }
    }
    let (solutions, preprocessed) = solve_player(player, requests, SOLVER_JS_STACK_BYTES)?;
    if let Some(preprocessed) = preprocessed {
        let record = remember(PlayerRecord {
            url: url.into(),
            signature_timestamp: super::bootstrap::timestamp_from_script(player),
            preprocessed: Some(preprocessed.into()),
        });
        persist(&record);
    }
    Ok(solutions)
}

pub(super) async fn solve(
    http: &FallbackHttp,
    player_url: &str,
    signatures: Vec<String>,
    throttling: Vec<String>,
) -> AppResult<ChallengeSolutions> {
    if signatures.is_empty() && throttling.is_empty() {
        return Ok(ChallengeSolutions::default());
    }
    if signatures.len() > MAX_CHALLENGES || throttling.len() > MAX_CHALLENGES {
        return Err(AppError::Api(
            "YouTube JS challenge input exceeds limits".into(),
        ));
    }
    let url = trusted_player_script_url(player_url)
        .ok_or_else(|| AppError::Api("Untrusted YouTube player script URL".into()))?;
    let started = Instant::now();
    let requests = Arc::new(challenge_requests(&signatures, &throttling));
    if let Some(preprocessed) = cached_record(&url).and_then(|record| record.preprocessed) {
        let fast_requests = requests.clone();
        match on_solver_thread(move || solve_preprocessed(&preprocessed, &fast_requests)).await {
            Ok(solutions) => {
                log::info!(
                    target: "youtube-playback",
                    "challenge solved mode=preprocessed, elapsed_ms={}",
                    started.elapsed().as_millis(),
                );
                return Ok(solutions);
            }
            Err(error) => {
                log::warn!(
                    target: "youtube-playback",
                    "preprocessed challenge failed, reparsing player: {error}"
                );
                forget_preprocessed(&url);
            }
        }
    }
    let script = player_script(http, &url).await?;
    let fetched_ms = started.elapsed().as_millis();
    let result = on_solver_thread(move || preprocess_and_solve(&url, &script, &requests)).await;
    match &result {
        Ok(_) => log::info!(
            target: "youtube-playback",
            "challenge solved mode=player, script_ms={fetched_ms}, elapsed_ms={}",
            started.elapsed().as_millis(),
        ),
        Err(error) => log::warn!(
            target: "youtube-playback",
            "challenge failed script_ms={fetched_ms}, elapsed_ms={}: {error}",
            started.elapsed().as_millis(),
        ),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"(function(){function F(a,b,c){var x={s:c,n:""};var p={set:function(k,v){x[k]=v;},get:function(k){return x[k];},clone:function(){return this;},solve:function(){x.s=x.s?x.s.split("").reverse().join(""):x.s;x.n=x.n?x.n.split("").reverse().join(""):x.n;}};var u=Object.create(p);u.set("alr","yes");return u;}}).call(this);"#;

    fn on_test_solver_thread<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> T {
        spawn_solver_thread(SOLVER_THREAD_STACK_BYTES, job)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn remote_script_host_and_path_are_restricted() {
        assert!(trusted_player_script_url("/s/player/abc/base.js").is_some());
        for url in [
            "https://youtube.com.evil.test/s/player/a.js",
            "http://www.youtube.com/s/player/a.js",
            "https://www.youtube.com/evil.js",
            "https://user@www.youtube.com/s/player/a.js",
            "https://www.youtube.com:8443/s/player/a.js",
        ] {
            assert!(trusted_player_script_url(url).is_none());
        }
    }

    #[test]
    fn sandbox_has_no_host_io_and_interrupts_infinite_loop() {
        on_test_solver_thread(|| {
            let limits = (SOLVE_MEMORY_BYTES, SOLVER_JS_STACK_BYTES);
            assert_eq!(evaluate("JSON.stringify([typeof fetch, typeof require, typeof Deno, typeof __TAURI_INTERNALS__])", Duration::from_secs(1), limits.0, limits.1).unwrap(), "[\"undefined\",\"undefined\",\"undefined\",\"undefined\"]");
            assert!(evaluate("while(true){}", Duration::from_millis(30), limits.0, limits.1).is_err());
        });
    }

    #[test]
    fn deep_recursion_reports_a_js_error_instead_of_overflowing_the_thread() {
        let error = on_test_solver_thread(|| {
            evaluate(
                "function f(n){return n?f(n-1)+1:0} String(f(1e9))",
                Duration::from_secs(20),
                SOLVE_MEMORY_BYTES,
                4 * 1024 * 1024,
            )
        })
        .unwrap_err();
        assert!(error.to_string().contains("RangeError"), "{error}");
    }

    #[test]
    fn bundled_ejs_resolves_and_reuses_preprocessed_player_fixture() {
        let (full, reused) = on_test_solver_thread(|| {
            let requests = challenge_requests(&["abc".into()], &["xyz".into()]);
            let (full, preprocessed) =
                solve_player(FIXTURE, &requests, SOLVER_JS_STACK_BYTES).unwrap();
            let preprocessed = preprocessed.expect("preprocessed player");
            let reused = solve_preprocessed(&preprocessed, &requests).unwrap();
            (full, reused)
        });
        for solutions in [&full, &reused] {
            assert_eq!(
                solutions.signatures.get("abc").map(String::as_str),
                Some("cba")
            );
            assert_eq!(
                solutions.throttling.get("xyz").map(String::as_str),
                Some("zyx")
            );
        }
    }

    #[test]
    fn player_snapshot_round_trips_and_rejects_foreign_or_stale_entries() {
        let directory = tempfile::tempdir().unwrap();
        let url = "https://www.youtube.com/s/player/abc/base.js";
        let record = PlayerRecord {
            url: url.into(),
            signature_timestamp: Some(20_000),
            preprocessed: Some("_result.n=v=>v".into()),
        };
        save_snapshot(directory.path(), &record, 1_000).unwrap();
        let path = snapshot_path(directory.path(), url);
        let loaded = load_snapshot(&path, url, 2_000).unwrap();
        assert_eq!(loaded.signature_timestamp, Some(20_000));
        assert_eq!(loaded.preprocessed.as_deref(), Some("_result.n=v=>v"));
        assert!(load_snapshot(&path, "https://www.youtube.com/s/player/other/base.js", 2_000).is_none());
        assert!(load_snapshot(&path, url, 999).is_none());
        assert!(load_snapshot(&path, url, 1_000 + SNAPSHOT_TTL_MS).is_none());
        let mut snapshot: PlayerSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        snapshot.solver = "older-solver".into();
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(load_snapshot(&path, url, 2_000).is_none());
    }

    #[test]
    fn player_snapshots_keep_only_the_newest_versions() {
        let directory = tempfile::tempdir().unwrap();
        for version in 0..5 {
            let record = PlayerRecord {
                url: format!("https://www.youtube.com/s/player/v{version}/base.js"),
                signature_timestamp: Some(version),
                preprocessed: None,
            };
            save_snapshot(directory.path(), &record, 1_000).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        let names = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), SNAPSHOT_LIMIT, "{names:?}");
        assert!(names.contains(
            &snapshot_path(directory.path(), "https://www.youtube.com/s/player/v4/base.js")
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ));
    }

    #[test]
    #[ignore = "set NERI_PLAYER_JS to a downloaded base.js to measure the real solver"]
    fn real_player_script_benchmark() {
        let Some(path) = std::env::var_os("NERI_PLAYER_JS") else {
            return;
        };
        let player = std::fs::read_to_string(path).unwrap();
        let signature = "AOq0QJ8wRQIhAKz3rNb6mP2v0yX1bD9cS7eF4gH5iJ6kL7mN8oP9qR0sAiBtU1vW2xY3zA4bC5dE6fG7hI8jK9lM0nO1pQ2rS3tU4vW5x".to_string();
        let throttling = "kQ9xY2zW8vU7tS6r".to_string();
        let requests = challenge_requests(&[signature.clone()], &[throttling.clone()]);
        for stack_mb in [8usize, 16, 32, 64, 128, 240] {
            let player = player.clone();
            let requests = requests.clone();
            let started = Instant::now();
            let result = spawn_solver_thread((stack_mb + 16) << 20, move || {
                solve_player(&player, &requests, stack_mb << 20)
            })
            .unwrap()
            .join()
            .unwrap();
            match result {
                Ok((solutions, preprocessed)) => {
                    let preprocessed = preprocessed.unwrap_or_default();
                    eprintln!(
                        "stack={stack_mb}MB full_ms={} sig={} n={} preprocessed_bytes={}",
                        started.elapsed().as_millis(),
                        solutions.signatures.contains_key(&signature),
                        solutions.throttling.contains_key(&throttling),
                        preprocessed.len(),
                    );
                    let requests = challenge_requests(&[signature.clone()], &[throttling.clone()]);
                    let fast_started = Instant::now();
                    let fast = on_test_solver_thread(move || {
                        solve_preprocessed(&preprocessed, &requests)
                    })
                    .unwrap();
                    eprintln!(
                        "preprocessed_ms={} sig={} n={}",
                        fast_started.elapsed().as_millis(),
                        fast.signatures.contains_key(&signature),
                        fast.throttling.contains_key(&throttling),
                    );
                    return;
                }
                Err(error) => eprintln!(
                    "stack={stack_mb}MB failed after {}ms: {error}",
                    started.elapsed().as_millis()
                ),
            }
        }
        panic!("no stack size solved the player");
    }
}

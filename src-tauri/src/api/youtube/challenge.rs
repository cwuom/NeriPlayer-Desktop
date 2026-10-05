use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime};
use serde_json::{json, Value};

use crate::api::transport::FallbackHttp;
use crate::error::{AppError, AppResult};

const MAX_SCRIPT_BYTES: usize = 8 * 1024 * 1024;
const SOLVE_TIMEOUT: Duration = Duration::from_secs(8);
const SCRIPT_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const LIB: &str = include_str!("assets/yt.solver.lib.min.js");
const CORE: &str = include_str!("assets/yt.solver.core.min.js");

struct CachedScript {
    url: String,
    script: String,
    fetched_at: Instant,
}

static SCRIPT_CACHE: LazyLock<Mutex<VecDeque<CachedScript>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

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

pub(super) async fn player_script(http: &FallbackHttp, url: &str) -> AppResult<String> {
    let url = trusted_player_script_url(url)
        .ok_or_else(|| AppError::Api("Untrusted YouTube player script URL".into()))?;
    if let Ok(cache) = SCRIPT_CACHE.lock() {
        if let Some(entry) = cache
            .iter()
            .find(|entry| entry.url == url && entry.fetched_at.elapsed() < SCRIPT_TTL)
        {
            return Ok(entry.script.clone());
        }
    }
    let mut response = http
        .send(|client| client.get(&url))
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
    let script = String::from_utf8(bytes)
        .map_err(|_| AppError::Api("YouTube player script is not UTF-8".into()))?;
    if let Ok(mut cache) = SCRIPT_CACHE.lock() {
        cache.retain(|entry| entry.url != url);
        cache.push_back(CachedScript {
            url,
            script: script.clone(),
            fetched_at: Instant::now(),
        });
        while cache.len() > 2 {
            cache.pop_front();
        }
    }
    Ok(script)
}

fn evaluate(script: &str, budget: Duration) -> AppResult<String> {
    let runtime =
        Runtime::new().map_err(|_| AppError::Api("YouTube JS runtime unavailable".into()))?;
    runtime.set_memory_limit(64 * 1024 * 1024);
    runtime.set_max_stack_size(1024 * 1024);
    let deadline = Instant::now() + budget;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime)
        .map_err(|_| AppError::Api("YouTube JS context unavailable".into()))?;
    // 不暴露文件、网络或 IPC 能力，远端脚本只处理 challenge 字符串
    context.with(|ctx| {
        ctx.eval::<String, _>(script).map_err(|_| {
            AppError::Api("YouTube JS challenge evaluation failed or exceeded limits".into())
        })
    })
}

fn solve_script(
    player: &str,
    signatures: &[String],
    throttling: &[String],
) -> AppResult<ChallengeSolutions> {
    if player.len() > MAX_SCRIPT_BYTES || signatures.len() > 64 || throttling.len() > 64 {
        return Err(AppError::Api(
            "YouTube JS challenge input exceeds limits".into(),
        ));
    }
    let mut requests = Vec::new();
    if !signatures.is_empty() {
        requests.push(json!({"type": "sig", "challenges": signatures}));
    }
    if !throttling.is_empty() {
        requests.push(json!({"type": "n", "challenges": throttling}));
    }
    let input = json!({"type": "player", "player": player, "requests": requests});
    let source =
        format!("{LIB}\nObject.assign(globalThis, lib);\n{CORE}\nJSON.stringify(jsc({input}));");
    let output = evaluate(&source, SOLVE_TIMEOUT)?;
    let value: Value = serde_json::from_str(&output)
        .map_err(|_| AppError::Api("YouTube JS challenge returned invalid JSON".into()))?;
    let mut solutions = ChallengeSolutions::default();
    let responses = value["responses"]
        .as_array()
        .ok_or_else(|| AppError::Api("YouTube JS challenge has no result".into()))?;
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

pub(super) async fn solve(
    http: &FallbackHttp,
    player_url: &str,
    signatures: Vec<String>,
    throttling: Vec<String>,
) -> AppResult<ChallengeSolutions> {
    if signatures.is_empty() && throttling.is_empty() {
        return Ok(ChallengeSolutions::default());
    }
    let player = player_script(http, player_url).await?;
    tokio::task::spawn_blocking(move || solve_script(&player, &signatures, &throttling))
        .await
        .map_err(|_| AppError::Api("YouTube JS challenge task cancelled".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(evaluate("JSON.stringify([typeof fetch, typeof require, typeof Deno, typeof __TAURI_INTERNALS__])", Duration::from_secs(1)).unwrap(), "[\"undefined\",\"undefined\",\"undefined\",\"undefined\"]");
        assert!(evaluate("while(true){}", Duration::from_millis(30)).is_err());
    }

    #[test]
    fn bundled_ejs_resolves_signature_and_throttling_from_player_fixture() {
        let fixture = r#"(function(){function F(a,b,c){var x={s:c,n:""};var p={set:function(k,v){x[k]=v;},get:function(k){return x[k];},clone:function(){return this;},solve:function(){x.s=x.s?x.s.split("").reverse().join(""):x.s;x.n=x.n?x.n.split("").reverse().join(""):x.n;}};var u=Object.create(p);u.set("alr","yes");return u;}}).call(this);"#;
        let result = solve_script(fixture, &["abc".into()], &["xyz".into()]).unwrap();
        assert_eq!(
            result.signatures.get("abc").map(String::as_str),
            Some("cba")
        );
        assert_eq!(
            result.throttling.get("xyz").map(String::as_str),
            Some("zyx")
        );
    }
}

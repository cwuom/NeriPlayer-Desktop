use std::collections::HashSet;

use super::archive::{
    self,
    approval::{self, SyncProtocolUpgrade},
};
use super::github_api::{GitHubApiClient, GitHubHead};
use super::models::{GitHubSyncConfig, SyncData, WebDavSyncConfig};
use super::webdav_archive::{self, Lease, RemoteFile, WebDavArchiveClient};
use super::{manager, merge, serializer};
use crate::error::{AppError, AppResult};

const RETRIES: usize = 3;
const LEGACY_BYTES: usize = 12 * 1024 * 1024;

struct Snapshot {
    data: SyncData,
    paths: HashSet<String>,
    protocol: u8,
    challenge: Option<SyncProtocolUpgrade>,
}

pub(crate) struct Completed {
    pub merged: SyncData,
    pub remote: Option<SyncData>,
    pub version: String,
    pub scope: String,
    pub uploaded: bool,
}

fn github_target(config: &GitHubSyncConfig, head: &GitHubHead) -> String {
    format!(
        "https://github.com/{}/{}@{}#repository={}&credential={}",
        config.owner,
        config.repo,
        head.branch,
        head.repository_id,
        archive::digest(config.token.as_bytes())
    )
}

async fn github_snapshot(
    api: &GitHubApiClient,
    config: &GitHubSyncConfig,
    head: &GitHubHead,
) -> AppResult<Option<Snapshot>> {
    let content = api
        .get_file_at_ref(
            &config.owner,
            &config.repo,
            archive::MANIFEST_FILE,
            &head.sha,
            archive::MAX_OBJECT_BYTES,
        )
        .await?;
    if let Some(content) = content {
        let loaded = archive::load(&content, |path| async move {
            api.get_file_at_ref(
                &config.owner,
                &config.repo,
                &path,
                &head.sha,
                archive::MAX_OBJECT_BYTES,
            )
            .await?
            .ok_or_else(|| AppError::Other("GitHub archive closure has a missing object".into()))
        })
        .await?;
        let challenge = (loaded.protocol != 4).then(|| {
            SyncProtocolUpgrade::new(
                "github",
                github_target(config, head),
                &content,
                loaded.protocol,
            )
        });
        return Ok(Some(Snapshot {
            data: loaded.data,
            paths: loaded.paths,
            protocol: loaded.protocol,
            challenge,
        }));
    }
    for filename in [
        serializer::get_filename(config.data_saver),
        serializer::get_filename(!config.data_saver),
    ] {
        if let Some(content) = api
            .get_file_at_ref(
                &config.owner,
                &config.repo,
                filename,
                &head.sha,
                LEGACY_BYTES,
            )
            .await?
        {
            let data = serializer::deserialize(&content)?;
            let challenge = Some(SyncProtocolUpgrade::new(
                "github",
                github_target(config, head),
                &content,
                0,
            ));
            return Ok(Some(Snapshot {
                data,
                paths: HashSet::new(),
                protocol: 0,
                challenge,
            }));
        }
    }
    Ok(None)
}

async fn webdav_snapshot(
    api: &WebDavArchiveClient,
    lease: Option<&Lease>,
) -> AppResult<(Option<RemoteFile>, Option<Snapshot>)> {
    if let Some(file) = api
        .get(archive::MANIFEST_FILE, lease, archive::MAX_OBJECT_BYTES)
        .await?
    {
        let loaded = archive::load(&file.content, |path| async move {
            api.get(&path, lease, archive::MAX_OBJECT_BYTES)
                .await?
                .map(|file| file.content)
                .ok_or_else(|| {
                    AppError::Other("WebDAV archive closure has a missing object".into())
                })
        })
        .await?;
        let challenge = (loaded.protocol != 4).then(|| {
            SyncProtocolUpgrade::new("webdav", api.target(), &file.content, loaded.protocol)
        });
        let snapshot = Snapshot {
            data: loaded.data,
            paths: loaded.paths,
            protocol: loaded.protocol,
            challenge,
        };
        return Ok((Some(file), Some(snapshot)));
    }
    if let Some(file) = api.get("neriplayer-sync.json", lease, LEGACY_BYTES).await? {
        let data = serializer::deserialize(&file.content)?;
        let challenge = Some(SyncProtocolUpgrade::new(
            "webdav",
            api.target(),
            &file.content,
            0,
        ));
        return Ok((
            None,
            Some(Snapshot {
                data,
                paths: HashSet::new(),
                protocol: 0,
                challenge,
            }),
        ));
    }
    Ok((None, None))
}

pub async fn inspect_github_upgrade(
    http: &reqwest::Client,
    config: &GitHubSyncConfig,
) -> AppResult<SyncProtocolUpgrade> {
    let api = GitHubApiClient::new(http, &config.token);
    let head = api.get_repository_head(&config.owner, &config.repo).await?;
    github_snapshot(&api, config, &head)
        .await?
        .and_then(|snapshot| snapshot.challenge)
        .ok_or_else(|| {
            AppError::Other("Current GitHub target does not require a protocol upgrade".into())
        })
}

pub async fn inspect_webdav_upgrade(
    http: &reqwest::Client,
    config: &WebDavSyncConfig,
) -> AppResult<SyncProtocolUpgrade> {
    let api = WebDavArchiveClient::new(http, config)?;
    webdav_snapshot(&api, None)
        .await?
        .1
        .and_then(|snapshot| snapshot.challenge)
        .ok_or_else(|| {
            AppError::Other("Current WebDAV target does not require a protocol upgrade".into())
        })
}

async fn prepare(data: &SyncData) -> AppResult<(archive::PreparedArchive, SyncData)> {
    let prepared = archive::prepare(data, Some(uuid::Uuid::new_v4().to_string()))?;
    let checked =
        archive::load(&prepared.content, |path| {
            std::future::ready(
                prepared.objects.get(&path).cloned().ok_or_else(|| {
                    AppError::Other("Prepared archive closure missing object".into())
                }),
            )
        })
        .await?;
    Ok((prepared, checked.data))
}

pub(super) async fn github(
    http: &reqwest::Client,
    config: &GitHubSyncConfig,
    local: &SyncData,
    epoch: u64,
) -> AppResult<Completed> {
    let api = GitHubApiClient::new(http, &config.token);
    for attempt in 0..=RETRIES {
        manager::ensure_local_playlist_epoch(epoch)?;
        let head = api.get_repository_head(&config.owner, &config.repo).await?;
        let snapshot = github_snapshot(&api, config, &head).await?;
        if let Some(challenge) = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.challenge.as_ref())
        {
            approval::require(challenge)?;
        }
        let scope = format!(
            "github-{}",
            archive::digest(github_target(config, &head).as_bytes())
        );
        let base = manager::load_base_snapshot(&scope)?;
        let merged = snapshot
            .as_ref()
            .map(|snapshot| {
                merge::three_way_merge(local, &snapshot.data, config.last_sync_time, &base)
            })
            .unwrap_or_else(|| local.normalized_for_sync());
        let upload = snapshot.as_ref().is_none_or(|snapshot| {
            snapshot.protocol != 4 || merge::has_data_changed(&snapshot.data, &merged)
        });
        if !upload {
            return Ok(Completed {
                merged,
                remote: snapshot.map(|snapshot| snapshot.data),
                version: head.sha,
                scope,
                uploaded: false,
            });
        }
        let (prepared, merged) = prepare(&merged).await?;
        manager::ensure_local_playlist_epoch(epoch)?;
        let verified = snapshot
            .as_ref()
            .map(|snapshot| snapshot.paths.clone())
            .unwrap_or_default();
        match api
            .publish_archive(&config.owner, &config.repo, &head, &prepared, &verified)
            .await
        {
            Ok(version) => {
                let committed = api
                    .get_file_at_ref(
                        &config.owner,
                        &config.repo,
                        archive::MANIFEST_FILE,
                        &version,
                        archive::MAX_OBJECT_BYTES,
                    )
                    .await?
                    .ok_or_else(|| AppError::Other("Committed GitHub manifest missing".into()))?;
                if committed != prepared.content {
                    return Err(AppError::Other(
                        "GitHub publication content mismatch".into(),
                    ));
                }
                let loaded = archive::load(&committed, |path| {
                    let version = &version;
                    let api = &api;
                    async move {
                        api.get_file_at_ref(
                            &config.owner,
                            &config.repo,
                            &path,
                            version,
                            archive::MAX_OBJECT_BYTES,
                        )
                        .await?
                        .ok_or_else(|| {
                            AppError::Other("Published GitHub closure missing object".into())
                        })
                    }
                })
                .await?;
                if loaded.paths != prepared.objects.keys().cloned().collect() {
                    return Err(AppError::Other(
                        "GitHub publication closure mismatch".into(),
                    ));
                }
                if let Some(challenge) = snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.challenge.as_ref())
                {
                    approval::consume(challenge)?;
                }
                return Ok(Completed {
                    merged,
                    remote: snapshot.map(|snapshot| snapshot.data),
                    version,
                    scope,
                    uploaded: true,
                });
            }
            Err(error) if error.is_content_conflict() && attempt < RETRIES => {
                tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt as u64 + 1)))
                    .await
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Other(
        "GitHub archive conflict retry budget exhausted".into(),
    ))
}

pub(super) async fn webdav(
    http: &reqwest::Client,
    config: &WebDavSyncConfig,
    local: &SyncData,
    epoch: u64,
) -> AppResult<Completed> {
    webdav_with_epoch_guard(http, config, local, || {
        manager::ensure_local_playlist_epoch(epoch)
    })
    .await
}

async fn webdav_with_epoch_guard(
    http: &reqwest::Client,
    config: &WebDavSyncConfig,
    local: &SyncData,
    ensure_epoch: impl Fn() -> AppResult<()>,
) -> AppResult<Completed> {
    let api = WebDavArchiveClient::new(http, config)?;
    let scope = format!("webdav-{}", api.scope());
    let base = manager::load_base_snapshot(&scope)?;
    let lease = api.acquire_lease().await?;
    let mut approved_upgrade = None;
    let operation = async {
        for attempt in 0..=RETRIES {
            ensure_epoch()?;
            let (observed, snapshot) = webdav_snapshot(&api, lease.as_ref()).await?;
            if let Some(challenge) = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.challenge.as_ref())
            {
                approval::require(challenge)?;
            }
            if lease.is_none()
                && snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.protocol == 0)
            {
                // 新清单的 ETag 不能约束旧 JSON 写入者，单文件迁移必须锁住整个集合
                return Err(AppError::Other(
                    "Legacy WebDAV upgrade requires a finite collection lease".into(),
                ));
            }
            let merged = snapshot
                .as_ref()
                .map(|snapshot| {
                    merge::three_way_merge(local, &snapshot.data, config.last_sync_time, &base)
                })
                .unwrap_or_else(|| local.normalized_for_sync());
            let upload = snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.protocol != 4 || merge::has_data_changed(&snapshot.data, &merged)
            });
            let verified = snapshot
                .as_ref()
                .map(|snapshot| snapshot.paths.clone())
                .unwrap_or_default();
            if !upload {
                return Ok((
                    Completed {
                        merged,
                        remote: snapshot.map(|snapshot| snapshot.data),
                        version: observed.as_ref().unwrap().fingerprint.clone(),
                        scope: scope.clone(),
                        uploaded: false,
                    },
                    observed.unwrap(),
                    verified,
                ));
            }
            let (prepared, merged) = prepare(&merged).await?;
            ensure_epoch()?;
            // 旧单文件仍是迁移依据，发布前确认它没有被旧客户端改写
            if snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.protocol == 0)
            {
                let legacy = api
                    .get("neriplayer-sync.json", lease.as_ref(), LEGACY_BYTES)
                    .await?
                    .ok_or_else(|| {
                        AppError::Other("Legacy WebDAV source disappeared during upgrade".into())
                    })?;
                let current = SyncProtocolUpgrade::new("webdav", api.target(), &legacy.content, 0);
                if snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.challenge.as_ref())
                    != Some(&current)
                {
                    return Err(AppError::Other(
                        "Legacy WebDAV source changed; refresh upgrade confirmation".into(),
                    ));
                }
            }
            match api
                .put_archive(&prepared, observed.as_ref(), &verified, lease.as_ref())
                .await
            {
                Ok(published) => {
                    let loaded = archive::load(&published.content, |path| {
                        let api = &api;
                        let lease = lease.as_ref();
                        async move {
                            api.get(&path, lease, archive::MAX_OBJECT_BYTES)
                                .await?
                                .map(|file| file.content)
                                .ok_or_else(|| {
                                    AppError::Other(
                                        "Published WebDAV closure missing object".into(),
                                    )
                                })
                        }
                    })
                    .await?;
                    if loaded.paths != prepared.objects.keys().cloned().collect() {
                        return Err(AppError::Other(
                            "WebDAV publication closure mismatch".into(),
                        ));
                    }
                    let latest = api
                        .get(
                            archive::MANIFEST_FILE,
                            lease.as_ref(),
                            archive::MAX_OBJECT_BYTES,
                        )
                        .await?
                        .ok_or_else(|| {
                            AppError::Other("Published WebDAV manifest missing".into())
                        })?;
                    if latest.fingerprint != published.fingerprint || latest.etag != published.etag
                    {
                        return Err(AppError::Other(
                            "WebDAV publication changed during validation".into(),
                        ));
                    }
                    approved_upgrade = snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.challenge.clone());
                    return Ok((
                        Completed {
                            merged,
                            remote: snapshot.map(|snapshot| snapshot.data),
                            version: published.fingerprint.clone(),
                            scope: scope.clone(),
                            uploaded: true,
                        },
                        published,
                        loaded.paths,
                    ));
                }
                Err(error)
                    if webdav_archive::is_conflict(&error)
                        && lease.is_none()
                        && attempt < RETRIES =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt as u64 + 1)))
                        .await
                }
                Err(error) => return Err(error),
            }
        }
        Err(AppError::Other(
            "WebDAV archive conflict retry budget exhausted".into(),
        ))
    }
    .await;
    let operation = match operation {
        Ok((mut completed, published, paths)) => {
            match api.maintain_gc(&published, &paths, lease.as_ref()).await {
                Ok(current) => {
                    completed.version = current.fingerprint.clone();
                    Ok((completed, current, paths))
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    let released = if let Some(lease) = lease.as_ref() {
        lease.release().await
    } else {
        Ok(())
    };
    let (completed, published, paths) = operation?;
    if let Err(error) = released {
        if webdav_archive::is_recoverable_release_failure(&error) {
            api.verify_release_recovery(&published, &paths).await?;
        } else {
            return Err(error);
        }
    }
    if let Some(challenge) = approved_upgrade {
        approval::consume(&challenge)?;
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct LegacyServerResult {
        requests: Vec<Vec<u8>>,
        files: BTreeMap<String, Vec<u8>>,
    }

    async fn legacy_server(
        finite_lease: bool,
    ) -> (
        reqwest::Client,
        WebDavSyncConfig,
        SyncProtocolUpgrade,
        tokio::task::JoinHandle<LegacyServerResult>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = WebDavSyncConfig {
            server_url: format!("http://{address}/"),
            username: uuid::Uuid::new_v4().to_string(),
            password: "fixture-password".into(),
            ..Default::default()
        };
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let legacy = serde_json::to_vec(&SyncData::default()).unwrap();
        let api = WebDavArchiveClient::new(&http, &config).unwrap();
        let challenge = SyncProtocolUpgrade::new("webdav", api.target(), &legacy, 0);
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut files = BTreeMap::from([("neriplayer-sync.json".to_string(), legacy)]);
            loop {
                let (mut socket, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                let end = loop {
                    let mut chunk = [0_u8; 8192];
                    let count = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        socket.read(&mut chunk),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert!(count > 0, "fixture request ended before its body");
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request
                        .windows(4)
                        .position(|bytes| bytes == b"\r\n\r\n")
                        .map(|index| index + 4)
                    {
                        let length = String::from_utf8_lossy(&request[..end])
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if request.len() >= end + length {
                            break end;
                        }
                    }
                };
                let header = String::from_utf8_lossy(&request[..end]);
                let mut first = header.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap();
                let path = first.next().unwrap().trim_start_matches('/');
                let (status, extra, body) = match method {
                    "LOCK" if finite_lease => ("200 OK", "Lock-Token: <12345>\r\n", b"<d:prop xmlns:d=\"DAV:\"><d:lockdiscovery><d:activelock><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:depth>infinity</d:depth><d:timeout>Second-300</d:timeout><d:locktoken><d:href>12345</d:href></d:locktoken></d:activelock></d:lockdiscovery></d:prop>".to_vec()),
                    "LOCK" => ("405 Method Not Allowed", "", Vec::new()),
                    "GET" => match files.get(path) {
                        Some(body) => ("200 OK", "ETag: W/\"fixture\"\r\n", body.clone()),
                        None => ("404 Not Found", "", Vec::new()),
                    },
                    "PUT" => {
                        files.insert(path.to_string(), request[end..].to_vec());
                        ("201 Created", "", Vec::new())
                    }
                    "UNLOCK" => ("204 No Content", "", Vec::new()),
                    _ => panic!("unexpected fixture method {method}"),
                };
                let finished = method == "UNLOCK"
                    || (!finite_lease && method == "GET" && path == "neriplayer-sync.json");
                let mut response = format!(
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(&body);
                requests.push(request);
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
                if finished {
                    break;
                }
            }
            LegacyServerResult { requests, files }
        });
        (http, config, challenge, task)
    }

    #[tokio::test]
    async fn legacy_webdav_upgrade_without_collection_lease_fails_before_any_put() {
        let (http, config, challenge, task) = legacy_server(false).await;
        approval::approve_verified(&challenge, &challenge).unwrap();
        let result = webdav_with_epoch_guard(&http, &config, &SyncData::default(), || Ok(())).await;
        let error = match result {
            Ok(_) => panic!("lease-free legacy upgrade must fail"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("finite collection lease"));
        let served = task.await.unwrap();
        assert_eq!(served.requests.len(), 3);
        assert!(served
            .requests
            .iter()
            .all(|request| !request.starts_with(b"PUT ")));
        assert!(!served.files.contains_key(archive::MANIFEST_FILE));
        approval::consume(&challenge).unwrap();
    }

    #[tokio::test]
    async fn legacy_webdav_upgrade_with_finite_lease_publishes_and_validates_complete_archive() {
        let (http, config, challenge, task) = legacy_server(true).await;
        approval::approve_verified(&challenge, &challenge).unwrap();
        let completed = webdav_with_epoch_guard(&http, &config, &SyncData::default(), || Ok(()))
            .await
            .unwrap();
        assert!(completed.uploaded);
        assert!(completed.remote.is_some());
        let served = task.await.unwrap();
        let published = &served.files[archive::MANIFEST_FILE];
        assert_eq!(completed.version, archive::digest(published));
        let loaded = archive::load(published, |path| {
            std::future::ready(
                served
                    .files
                    .get(&path)
                    .cloned()
                    .ok_or_else(|| AppError::Other("fixture closure missing".into())),
            )
        })
        .await
        .unwrap();
        assert_eq!(loaded.protocol, 4);
        assert!(loaded
            .paths
            .iter()
            .all(|path| served.files.contains_key(path)));
        let puts: Vec<_> = served
            .requests
            .iter()
            .filter(|request| request.starts_with(b"PUT "))
            .collect();
        assert!(!puts.is_empty());
        for request in puts {
            let header = String::from_utf8_lossy(request).to_ascii_lowercase();
            assert!(header.contains("if: <http://"));
            assert!(header.contains("(<12345>)"));
            assert!(header.contains("if-none-match: *\r\n"));
        }
        assert!(served.requests.last().unwrap().starts_with(b"UNLOCK "));
        assert!(approval::require(&challenge).is_err());
    }

    #[test]
    fn github_upgrade_permission_is_bound_to_credentials_without_exposing_them() {
        let mut config = GitHubSyncConfig {
            owner: uuid::Uuid::new_v4().to_string(),
            repo: "backup".into(),
            token: "fixture-first-credential".into(),
            ..Default::default()
        };
        let head = GitHubHead {
            branch: "main".into(),
            sha: "head".into(),
            repository_id: "repository".into(),
        };
        let original =
            SyncProtocolUpgrade::new("github", github_target(&config, &head), b"backup", 0);
        assert!(!original.target.contains(&config.token));
        approval::approve_verified(&original, &original).unwrap();
        config.token = "fixture-replaced-credential".into();
        let changed =
            SyncProtocolUpgrade::new("github", github_target(&config, &head), b"backup", 0);
        assert!(approval::require(&changed).is_err());
        assert!(approval::approve_verified(&original, &changed).is_err());
        approval::consume(&original).unwrap();
    }
}

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::NsReader;
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use url::Url;

use super::archive::{self, PreparedArchive};
use super::failure::SyncFailure;
use super::models::WebDavSyncConfig;
use crate::error::{AppError, AppResult};

const MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
const DIRECTORY_PROBE_BODY: &[u8] =
    b"<d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/></d:prop></d:propfind>";
const CONFLICT_PREFIX: &str = "WebDAV archive conflict";
static FINITE_LEASE_TARGETS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub fn is_conflict(error: &AppError) -> bool {
    matches!(error, AppError::Api(message) if message.starts_with(CONFLICT_PREFIX))
}

fn conflict(message: &str) -> AppError {
    AppError::Api(format!("{CONFLICT_PREFIX}: {message}"))
}
fn invalid(message: &str) -> AppError {
    AppError::Api(format!("WebDAV archive: {message}"))
}

pub fn is_recoverable_release_failure(error: &AppError) -> bool {
    match error {
        AppError::Network(error) => error.is_timeout() || error.is_connect() || error.is_request(),
        AppError::Api(message) => {
            message.starts_with("WebDAV archive: UNLOCK failed (")
                && message
                    .strip_prefix("WebDAV archive: UNLOCK failed (")
                    .and_then(|value| value.strip_suffix(')'))
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some_and(|status| status == 408 || status >= 500)
        }
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct RemoteFile {
    pub content: Vec<u8>,
    pub fingerprint: String,
    pub etag: Option<String>,
}

#[derive(Clone)]
pub struct WebDavArchiveClient {
    http: Client,
    collection: Url,
    username: String,
    password: String,
}

struct LeaseState {
    token: String,
    expires: Instant,
    closed: bool,
}
pub struct Lease {
    client: WebDavArchiveClient,
    state: Arc<tokio::sync::Mutex<LeaseState>>,
}

impl WebDavArchiveClient {
    pub fn new(http: &Client, config: &WebDavSyncConfig) -> AppResult<Self> {
        let mut collection =
            Url::parse(&config.server_url).map_err(|_| invalid("invalid server URL"))?;
        if !matches!(collection.scheme(), "http" | "https")
            || !collection.username().is_empty()
            || collection.password().is_some()
        {
            return Err(invalid(
                "server URL must be HTTP(S) without embedded credentials",
            ));
        }
        collection.set_fragment(None);
        {
            let mut segments = collection
                .path_segments_mut()
                .map_err(|_| invalid("invalid collection path"))?;
            segments.pop_if_empty();
            for part in config.base_path.split('/').filter(|part| !part.is_empty()) {
                if matches!(part, "." | "..") || part.contains('\\') {
                    return Err(invalid("invalid base path"));
                }
                segments.push(part);
            }
            segments.push("");
        }
        Ok(Self {
            http: http.clone(),
            collection,
            username: config.username.clone(),
            password: config.password.clone(),
        })
    }

    pub fn target(&self) -> String {
        let mut url = self
            .file_url(archive::MANIFEST_FILE)
            .unwrap_or_else(|_| self.collection.clone());
        url.set_fragment(Some(&format!(
            "account={}&credential={}",
            archive::digest(self.username.as_bytes()),
            archive::digest(self.password.as_bytes())
        )));
        url.to_string()
    }
    pub fn scope(&self) -> String {
        archive::digest(format!("{}|{}", self.collection, self.username).as_bytes())
    }
    fn file_url(&self, path: &str) -> AppResult<Url> {
        if path.is_empty()
            || path.contains('/')
            || path.contains('\\')
            || matches!(path, "." | "..")
        {
            return Err(invalid("invalid sibling file name"));
        }
        let mut url = self.collection.clone();
        url.path_segments_mut()
            .map_err(|_| invalid("invalid collection URL"))?
            .pop_if_empty()
            .push(path);
        Ok(url)
    }
    fn request(&self, method: Method, url: Url) -> RequestBuilder {
        self.http
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Cache-Control", "no-cache")
            .header("Pragma", "no-cache")
    }

    /// 旧版单文件能读到就说明目录可用；404 时 get 已经用 PROPFIND 确认过目录
    pub async fn validate_connection(&self) -> AppResult<()> {
        self.get("neriplayer-sync.json", None, MAX_RESPONSE_BYTES)
            .await
            .map(|_| ())
    }

    /// 用 Depth: 0 的 PROPFIND 确认同步目录存在且确实是目录（对齐 Android WebDavDirectoryProbe）
    async fn probe_directory(&self, lease: Option<&Lease>) -> AppResult<()> {
        let (status, body, _) = self
            .execute(
                Method::from_bytes(b"PROPFIND").unwrap(),
                self.collection.clone(),
                lease,
                Some(("Depth", "0")),
                Some(DIRECTORY_PROBE_BODY),
                64 * 1024,
            )
            .await?;
        directory_status(status, &body)
    }

    /// 写入返回 404/409 时多半是目录没了，先确认目录；目录还在时 409 按冲突处理
    async fn write_failure(
        &self,
        status: StatusCode,
        operation: &str,
        lease: Option<&Lease>,
    ) -> AppError {
        if matches!(status.as_u16(), 404 | 409) {
            if let Err(error) = self.probe_directory(lease).await {
                return error;
            }
        }
        if status == StatusCode::CONFLICT {
            conflict(&format!("{operation} conflict"))
        } else {
            status_error(status, operation)
        }
    }

    pub async fn acquire_lease(&self) -> AppResult<Option<Lease>> {
        let scope = self.scope();
        let known = known_lease_targets()?.contains(&scope);
        if self.collection.query().is_some() {
            if known {
                return Err(invalid(
                    "previously locked query target cannot downgrade to unlocked sync",
                ));
            }
            return Ok(None);
        }
        let start = Instant::now();
        let response=self.request(Method::from_bytes(b"LOCK").unwrap(),self.collection.clone()).header("Depth","infinity").header("Timeout","Second-300").header("Content-Type","application/xml; charset=utf-8").body("<d:lockinfo xmlns:d=\"DAV:\"><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype></d:lockinfo>").timeout(Duration::from_secs(60)).send().await.map_err(reqwest::Error::without_url)?;
        let status = response.status();
        if matches!(status.as_u16(), 405 | 501) && !known {
            return Ok(None);
        }
        let redirected = response.url() != &self.collection;
        let header_token = response
            .headers()
            .get("Lock-Token")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_header_token);
        let body = bounded_body(response, 64 * 1024).await?;
        let grant = if redirected {
            Err(invalid("LOCK redirected outside the collection"))
        } else if status != StatusCode::OK {
            Err(status_error(status, "LOCK"))
        } else {
            parse_grant(&body, header_token.as_deref(), &self.collection, None)
        };
        let (token, seconds) = match grant {
            Ok(grant) => grant,
            Err(error) => {
                let released = if matches!(status.as_u16(), 200 | 201) {
                    if let Some(token) = header_token {
                        self.unlock(&token).await.is_ok()
                    } else {
                        false
                    }
                } else {
                    false
                };
                if error.to_string().contains("unsafe lease duration") && released && !known {
                    return Ok(None);
                }
                return Err(error);
            }
        };
        let expires = start + Duration::from_secs(seconds);
        if Instant::now() >= expires {
            let _ = self.unlock(&token).await;
            return Err(invalid("lease expired during acquisition"));
        }
        if let Err(error) = mark_known_lease(&scope) {
            let _ = self.unlock(&token).await;
            return Err(error);
        }
        Ok(Some(Lease {
            client: self.clone(),
            state: Arc::new(tokio::sync::Mutex::new(LeaseState {
                token,
                expires,
                closed: false,
            })),
        }))
    }

    async fn unlock(&self, token: &str) -> AppResult<()> {
        let response = self
            .request(
                Method::from_bytes(b"UNLOCK").unwrap(),
                self.collection.clone(),
            )
            .header("Lock-Token", format!("<{token}>"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        if response.url() != &self.collection {
            return Err(invalid("UNLOCK redirected"));
        }
        if !matches!(response.status().as_u16(), 204 | 409) {
            return Err(status_error(response.status(), "UNLOCK"));
        }
        Ok(())
    }

    async fn execute(
        &self,
        method: Method,
        url: Url,
        lease: Option<&Lease>,
        condition: Option<(&str, &str)>,
        content: Option<&[u8]>,
        maximum: usize,
    ) -> AppResult<(StatusCode, Vec<u8>, Option<String>)> {
        let mut guard = if let Some(lease) = lease {
            Some(lease.state.lock().await)
        } else {
            None
        };
        if let Some(state) = guard.as_mut() {
            lease.unwrap().refresh_if_needed(state).await?;
        }
        let xml = method.as_str() == "PROPFIND";
        let mut request = self.request(method, url.clone());
        let timeout = if let Some(state) = guard.as_ref() {
            let remaining = state
                .expires
                .checked_duration_since(Instant::now())
                .and_then(|duration| duration.checked_sub(Duration::from_secs(5)))
                .ok_or_else(|| invalid("lease expired"))?;
            request = request.header("If", format!("<{}> (<{}>)", self.collection, state.token));
            remaining.min(Duration::from_secs(60))
        } else {
            Duration::from_secs(60)
        };
        if let Some((name, value)) = condition {
            request = request.header(name, value);
        }
        if let Some(content) = content {
            request = request
                .header(
                    "Content-Type",
                    if xml {
                        "application/xml; charset=utf-8"
                    } else {
                        "application/octet-stream"
                    },
                )
                .body(content.to_vec());
        }
        let response = request
            .timeout(timeout)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        if response.url() != &url {
            return Err(invalid("object request redirected"));
        }
        let status = response.status();
        if matches!(status.as_u16(), 412 | 423) {
            return Err(conflict("condition changed or lease lost"));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| strong_etag(value))
            .map(String::from);
        let body = bounded_body(response, maximum).await?;
        if guard
            .as_ref()
            .is_some_and(|state| state.closed || Instant::now() >= state.expires)
        {
            return Err(invalid("lease expired during request"));
        }
        Ok((status, body, etag))
    }

    pub async fn get(
        &self,
        path: &str,
        lease: Option<&Lease>,
        maximum: usize,
    ) -> AppResult<Option<RemoteFile>> {
        let (status, content, etag) = self
            .execute(
                Method::GET,
                self.file_url(path)?,
                lease,
                None,
                None,
                maximum,
            )
            .await?;
        if status == StatusCode::NOT_FOUND {
            // 目录不存在时远端看起来也是"空"的，首次同步会把它当成新仓库，必须先排除
            self.probe_directory(lease).await?;
            return Ok(None);
        }
        if !status.is_success() {
            return Err(status_error(status, "GET"));
        }
        Ok(Some(RemoteFile {
            fingerprint: archive::digest(&content),
            content,
            etag,
        }))
    }

    pub async fn put_archive(
        &self,
        prepared: &PreparedArchive,
        observed: Option<&RemoteFile>,
        verified_paths: &HashSet<String>,
        lease: Option<&Lease>,
    ) -> AppResult<RemoteFile> {
        // 持锁服务器也可能忽略条件头，先重新读取本次合并依据的清单
        if lease.is_some() {
            let current = self
                .get(archive::MANIFEST_FILE, lease, archive::MAX_OBJECT_BYTES)
                .await?;
            match (observed, current.as_ref()) {
                (None, None) => {}
                (Some(old), Some(current))
                    if old.fingerprint == current.fingerprint
                        && (old.etag.is_none() || old.etag == current.etag) => {}
                _ => return Err(conflict("manifest changed before publication")),
            }
        }
        if observed.is_some_and(|file| file.etag.is_none()) && lease.is_none() {
            return Err(SyncFailure::WebDavMissingCondition.into());
        }
        for (path, content) in &prepared.objects {
            if !archive::canonical_object_path(path) || content.len() > archive::MAX_OBJECT_BYTES {
                return Err(invalid("invalid upload object"));
            }
            if verified_paths.contains(path) {
                continue;
            }
            let result = self
                .execute(
                    Method::PUT,
                    self.file_url(path)?,
                    lease,
                    Some(("If-None-Match", "*")),
                    Some(content),
                    64 * 1024,
                )
                .await;
            let result = match result {
                Ok((status, _, _)) if !status.is_success() => {
                    Err(self.write_failure(status, "object PUT", lease).await)
                }
                result => result,
            };
            match result {
                Ok(_) => {}
                Err(error) if is_conflict(&error) => {
                    let existing = self
                        .get(path, lease, archive::MAX_OBJECT_BYTES)
                        .await?
                        .ok_or_else(|| conflict("object disappeared after create conflict"))?;
                    if existing.content != *content {
                        return Err(invalid("content addressed object was replaced"));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        let condition = if let Some(etag) = observed.and_then(|file| file.etag.as_deref()) {
            Some(("If-Match", etag))
        } else if observed.is_none() {
            Some(("If-None-Match", "*"))
        } else {
            None
        };
        let (status, _, _) = self
            .execute(
                Method::PUT,
                self.file_url(archive::MANIFEST_FILE)?,
                lease,
                condition,
                Some(&prepared.content),
                64 * 1024,
            )
            .await?;
        if !status.is_success() {
            return Err(self.write_failure(status, "manifest PUT", lease).await);
        }
        let published = self
            .get(archive::MANIFEST_FILE, lease, archive::MAX_OBJECT_BYTES)
            .await?
            .ok_or_else(|| invalid("published manifest missing"))?;
        if published.content != prepared.content {
            return Err(conflict(
                "publication was overwritten or conditional PUT ignored",
            ));
        }
        Ok(published)
    }

    pub async fn verify_release_recovery(
        &self,
        published: &RemoteFile,
        paths: &HashSet<String>,
    ) -> AppResult<()> {
        if published
            .etag
            .as_deref()
            .is_none_or(|etag| !strong_etag(etag))
        {
            return Err(invalid("release recovery requires a strong manifest ETag"));
        }
        let lease = self
            .acquire_lease()
            .await?
            .ok_or_else(|| invalid("release recovery requires a finite lease"))?;
        let result = async {
            let current = self
                .get(
                    archive::MANIFEST_FILE,
                    Some(&lease),
                    archive::MAX_OBJECT_BYTES,
                )
                .await?
                .ok_or_else(|| invalid("release recovery manifest missing"))?;
            if current.etag != published.etag || current.fingerprint != published.fingerprint {
                return Err(conflict("publication changed during release recovery"));
            }
            let lease_ref = &lease;
            let loaded = archive::load(&current.content, |path| async move {
                self.get(&path, Some(lease_ref), archive::MAX_OBJECT_BYTES)
                    .await?
                    .map(|file| file.content)
                    .ok_or_else(|| invalid("recovery closure missing object"))
            })
            .await?;
            if &loaded.paths != paths {
                return Err(invalid("release recovery closure mismatch"));
            }
            Ok(())
        }
        .await;
        let released = lease.release().await;
        result?;
        released
    }

    pub async fn maintain_gc(
        &self,
        published: &RemoteFile,
        paths: &HashSet<String>,
        retained: &HashSet<String>,
        lease: Option<&Lease>,
    ) -> AppResult<RemoteFile> {
        self.maintain_gc_at(
            published,
            paths,
            retained,
            lease,
            chrono::Utc::now().timestamp_millis(),
            super::webdav_gc::uptime_ms(),
        )
        .await
    }

    /// `retained` 是刚被替换的旧清单引用的对象：别的设备可能还在按旧清单读取，
    /// 这一轮不把它们当候选，保留期从头计算（对齐 Android journal.protect）
    async fn maintain_gc_at(
        &self,
        published: &RemoteFile,
        paths: &HashSet<String>,
        retained: &HashSet<String>,
        lease: Option<&Lease>,
        wall: i64,
        uptime: i64,
    ) -> AppResult<RemoteFile> {
        let mut current = published.clone();
        if lease.is_none() || self.collection.query().is_some() || published.etag.is_none() {
            return Ok(current);
        }
        let result=async {
            let listing_body=b"<d:propfind xmlns:d=\"DAV:\"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>";
            let (status,body,_)=self.execute(Method::from_bytes(b"PROPFIND").unwrap(),self.collection.clone(),lease,Some(("Depth","1")),Some(listing_body),512*1024).await?;
            if status!=StatusCode::MULTI_STATUS {return Err(status_error(status,"archive listing"));}
            let entries=parse_listing(&body,&self.collection)?;
            let path=gc_journal_path(&self.scope());
            let previous=match std::fs::read(&path) {
                Ok(bytes) if bytes.len()<=512*1024=>serde_json::from_slice::<super::webdav_gc::Journal>(&bytes).unwrap_or_default(),
                Ok(_)=>super::webdav_gc::Journal::default(),
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>super::webdav_gc::Journal::default(),
                Err(error)=>return Err(error.into()),
            };
            let protected:HashSet<String>=paths.union(retained).cloned().collect();
            let mut journal=previous.observe(&entries,&protected,wall,uptime);
            let eligible=journal.eligible();
            if !cfg!(test) {crate::fsutil::atomic_write(&path,serde_json::to_vec(&journal)?)?;}
            if eligible.is_empty() {return Ok(());}
            let refreshed=archive::PreparedArchive {content:archive::renew_publication_id(&current.content)?,objects:std::collections::BTreeMap::new()};
            current=self.put_archive(&refreshed,Some(&current),&HashSet::new(),lease).await?;
            if current.etag.is_none() {return Err(invalid("maintenance fence requires strong ETag"));}
            let checked=archive::load(&current.content,|object|async move {
                self.get(&object,lease,archive::MAX_OBJECT_BYTES).await?.map(|file|file.content).ok_or_else(||invalid("maintenance closure missing object"))
            }).await?;
            if &checked.paths!=paths {return Err(invalid("maintenance fence closure changed"));}
            for candidate in eligible {
                if paths.contains(&candidate.path) {return Err(invalid("maintenance candidate is referenced"));}
                let root=self.get(archive::MANIFEST_FILE,lease,archive::MAX_OBJECT_BYTES).await?.ok_or_else(||invalid("maintenance manifest missing"))?;
                if root.fingerprint!=current.fingerprint || root.etag!=current.etag {return Err(conflict("maintenance fence changed"));}
                match self.execute(Method::DELETE,self.file_url(&candidate.path)?,lease,Some(("If-Match",&candidate.etag)),None,64*1024).await {
                    Ok((status,_,_)) if matches!(status.as_u16(),200|204|404)=>journal.remove(&candidate.path),
                    Err(error) if is_conflict(&error)=>{},
                    Ok((status,_,_))=>return Err(status_error(status,"retired object DELETE")),
                    Err(error)=>return Err(error),
                }
            }
            if !cfg!(test) {crate::fsutil::atomic_write(&path,serde_json::to_vec(&journal)?)?;}
            Ok(())
        }.await;
        if let Err(error) = result {
            log::warn!(target:"sync","WebDAV archive maintenance deferred: {error}");
        }
        // 清理失败可保留旧对象，但不能把不确定的新发布当作已确认根
        let latest = self
            .get(archive::MANIFEST_FILE, lease, archive::MAX_OBJECT_BYTES)
            .await?
            .ok_or_else(|| invalid("manifest missing after maintenance"))?;
        if latest.etag != current.etag || latest.fingerprint != current.fingerprint {
            if !archive::same_closure(&latest.content, &published.content)? {
                return Err(conflict("manifest changed during maintenance"));
            }
            archive::load(&latest.content, |object| async move {
                self.get(&object, lease, archive::MAX_OBJECT_BYTES)
                    .await?
                    .map(|file| file.content)
                    .ok_or_else(|| invalid("maintenance closure missing object"))
            })
            .await?;
        }
        Ok(latest)
    }
}

fn gc_journal_path(scope: &str) -> std::path::PathBuf {
    let base = if cfg!(test) {
        std::env::temp_dir().join("neriplayer-webdav-test-journals")
    } else {
        dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("NeriPlayer")
    };
    base.join(format!("sync-webdav-gc-{scope}.json"))
}

/// Depth: 0 目录探测的结果（对齐 Android WebDavDirectoryProbe / WebDavDirectoryResponse）
fn directory_status(status: StatusCode, body: &[u8]) -> AppResult<()> {
    match status.as_u16() {
        207 => validate_collection(body),
        401 => Err(SyncFailure::WebDavAuth.into()),
        403 => Err(SyncFailure::WebDavAccessDenied.into()),
        404 => Err(SyncFailure::WebDavDirectoryNotFound.into()),
        200..=299 => Err(invalid("invalid directory response")),
        _ => Err(status_error(status, "collection probe")),
    }
}

fn validate_collection(bytes: &[u8]) -> AppResult<()> {
    let document = parse_xml(bytes)?;
    if !document.dav || document.name != "multistatus" {
        return Err(invalid("invalid directory response"));
    }
    let resource = required(&document, "response")?;
    if let Some(status) = child(resource, "status")? {
        let code = status
            .text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| invalid("malformed directory status"))?;
        if code == 404 {
            return Err(SyncFailure::WebDavDirectoryNotFound.into());
        }
        return Err(invalid(&format!("directory resource status {code}")));
    }
    let mut properties = resource
        .children
        .iter()
        .filter(|node| node.dav && node.name == "propstat")
        .filter_map(|node| {
            let property = child(node, "prop").ok().flatten()?;
            child(property, "resourcetype")
                .ok()
                .flatten()
                .map(|kind| (node, kind))
        });
    let (property, kind) = properties
        .next()
        .ok_or_else(|| invalid("directory response omits resource type"))?;
    if properties.next().is_some() {
        return Err(invalid("directory response has duplicate resource types"));
    }
    let code = required(property, "status")?
        .text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| invalid("malformed directory property status"))?;
    if !(200..300).contains(&code) {
        return Err(invalid(&format!("directory property status {code}")));
    }
    if child(kind, "collection")?.is_none() {
        return Err(SyncFailure::WebDavNotDirectory.into());
    }
    Ok(())
}

fn parse_listing(
    bytes: &[u8],
    root: &Url,
) -> AppResult<std::collections::BTreeMap<String, String>> {
    let document = parse_xml(bytes)?;
    if !document.dav || document.name != "multistatus" {
        return Err(invalid("invalid archive listing root"));
    }
    let mut entries = std::collections::BTreeMap::new();
    let mut seen = HashSet::new();
    let mut collection = false;
    for response in document
        .children
        .iter()
        .filter(|node| node.dav && node.name == "response")
    {
        let href = required(response, "href")?.text.trim();
        let url = root
            .join(href)
            .map_err(|_| invalid("invalid archive listing href"))?;
        if href != url.as_str() && href != url.path() {
            return Err(invalid("noncanonical archive listing href"));
        }
        if url.origin() != root.origin() || !seen.insert(url.to_string()) {
            return Err(invalid("duplicate or out of scope archive listing"));
        }
        if child(response, "status")?.is_some() {
            return Err(invalid("incomplete archive listing response"));
        }
        let mut successful = response
            .children
            .iter()
            .filter(|node| node.dav && node.name == "propstat")
            .filter(|node| {
                child(node, "status")
                    .ok()
                    .flatten()
                    .is_some_and(|status| status.text.split_whitespace().nth(1) == Some("200"))
            });
        let properties = required(
            successful
                .next()
                .ok_or_else(|| invalid("missing listing properties"))?,
            "prop",
        )?;
        if successful.next().is_some() {
            return Err(invalid("duplicate successful listing properties"));
        }
        let resource_type = required(properties, "resourcetype")?;
        let is_collection = child(resource_type, "collection")?.is_some();
        if &url == root {
            if !is_collection {
                return Err(invalid("archive root is not a collection"));
            }
            collection = true;
            continue;
        }
        let Some(name) = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
        else {
            return Err(invalid("missing listing filename"));
        };
        if !archive::canonical_object_path(name) {
            continue;
        }
        let mut expected = root.clone();
        expected
            .path_segments_mut()
            .map_err(|_| invalid("invalid listing root"))?
            .pop_if_empty()
            .push(name);
        if url != expected || url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("unexpected archive object path"));
        }
        if is_collection {
            continue;
        }
        let etag = required(properties, "getetag")?.text.trim();
        if !strong_etag(etag) {
            return Err(invalid("listed object has no strong ETag"));
        }
        entries.insert(name.to_string(), etag.to_string());
        if entries.len() > 1024 {
            return Err(invalid("archive listing exceeds maintenance budget"));
        }
    }
    if !collection {
        return Err(invalid("archive listing omits collection"));
    }
    Ok(entries)
}

impl Lease {
    async fn refresh_if_needed(&self, state: &mut LeaseState) -> AppResult<()> {
        let start = Instant::now();
        let remaining = state
            .expires
            .checked_duration_since(start)
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| invalid("lease expired"))?;
        if state.closed {
            return Err(invalid("lease closed"));
        }
        if remaining > Duration::from_secs(65) {
            return Ok(());
        }
        let response = self
            .client
            .request(
                Method::from_bytes(b"LOCK").unwrap(),
                self.client.collection.clone(),
            )
            .header(
                "If",
                format!("<{}> (<{}>)", self.client.collection, state.token),
            )
            .header("Timeout", "Second-300")
            .timeout(remaining.min(Duration::from_secs(60)))
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        if response.url() != &self.client.collection {
            return Err(invalid("lease refresh redirected"));
        }
        let status = response.status();
        let token = response
            .headers()
            .get("Lock-Token")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_header_token);
        let body = bounded_body(response, 64 * 1024).await?;
        if status != StatusCode::OK {
            return Err(status_error(status, "lease refresh"));
        }
        if Instant::now() >= state.expires {
            return Err(invalid("lease expired during refresh"));
        }
        let (_, seconds) = parse_grant(
            &body,
            token.as_deref(),
            &self.client.collection,
            Some(&state.token),
        )?;
        state.expires = start + Duration::from_secs(seconds);
        if Instant::now() >= state.expires {
            return Err(invalid("refreshed lease expired"));
        }
        Ok(())
    }
    pub async fn release(&self) -> AppResult<()> {
        let mut state = self.state.lock().await;
        if state.closed {
            return Ok(());
        }
        state.closed = true;
        self.client.unlock(&state.token).await
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_lock() {
            if state.closed {
                return;
            }
            state.closed = true;
            let token = state.token.clone();
            let client = self.client.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = client.unlock(&token).await;
                });
            }
        }
    }
}

fn known_lease_targets() -> AppResult<HashSet<String>> {
    let memory = FINITE_LEASE_TARGETS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = memory
        .lock()
        .map_err(|_| invalid("lease capability lock poisoned"))?;
    if guard.is_empty() && !cfg!(test) {
        *guard = crate::db::user_db()?.read(super::storage::load_lease_targets)?;
    }
    Ok(guard.clone())
}
fn mark_known_lease(scope: &str) -> AppResult<()> {
    let memory = FINITE_LEASE_TARGETS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut targets = memory
        .lock()
        .map_err(|_| invalid("lease capability lock poisoned"))?;
    targets.insert(scope.to_string());
    if !cfg!(test) {
        crate::db::user_db()?
            .write(|transaction| super::storage::save_lease_targets(transaction, &targets))?;
    }
    Ok(())
}

async fn bounded_body(mut response: reqwest::Response, maximum: usize) -> AppResult<Vec<u8>> {
    let maximum = maximum.min(MAX_RESPONSE_BYTES);
    if response
        .content_length()
        .is_some_and(|size| size > maximum as u64)
    {
        return Err(invalid("response exceeds budget"));
    }
    let mut output = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if chunk.len() > maximum.saturating_sub(output.len()) {
            return Err(invalid("response exceeds budget"));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}
fn status_error(status: StatusCode, operation: &str) -> AppError {
    match status.as_u16() {
        412 | 423 => conflict("condition changed or collection locked"),
        401 => SyncFailure::WebDavAuth.into(),
        403 => SyncFailure::WebDavAccessDenied.into(),
        code => invalid(&format!("{operation} failed ({code})")),
    }
}
pub fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && !value[1..value.len() - 1]
            .chars()
            .any(|character| character == '"' || character.is_control())
}
fn valid_token(token: &str) -> bool {
    token.len() <= 1024
        && !token.is_empty()
        && !token.chars().any(|character| {
            character.is_whitespace() || character.is_control() || matches!(character, '<' | '>')
        })
        && (token.bytes().all(|byte| byte.is_ascii_digit())
            || token.split_once(':').is_some_and(|(scheme, rest)| {
                !rest.is_empty()
                    && scheme
                        .bytes()
                        .next()
                        .is_some_and(|byte| byte.is_ascii_alphabetic())
                    && scheme.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-')
                    })
            }))
}
fn parse_header_token(header: &str) -> Option<String> {
    let header = header.trim();
    let token = if header.starts_with('<') || header.ends_with('>') {
        header.strip_prefix('<')?.strip_suffix('>')?
    } else {
        header
    };
    valid_token(token).then(|| token.to_string())
}

struct XmlNode {
    dav: bool,
    name: String,
    text: String,
    children: Vec<XmlNode>,
}
fn parse_xml(bytes: &[u8]) -> AppResult<XmlNode> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root = None;
    let mut count = 0;
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|_| invalid("malformed DAV XML"))?;
        match event {
            Event::Start(element) => {
                count += 1;
                if count > 8192 || stack.len() > 32 {
                    return Err(invalid("DAV XML exceeds structural budget"));
                }
                let dav =
                    matches!(namespace,ResolveResult::Bound(value) if value.as_ref()==b"DAV:");
                stack.push(XmlNode {
                    dav,
                    name: String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| invalid("malformed DAV XML end"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else if root.replace(node).is_some() {
                    return Err(invalid("multiple DAV XML roots"));
                }
            }
            Event::Text(text) => {
                let value = text
                    .decode()
                    .map_err(|_| invalid("invalid DAV XML encoding"))?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|_| invalid("invalid DAV XML entity"))?;
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err(invalid("text outside DAV root"));
                }
            }
            Event::CData(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text
                        .push_str(&text.decode().map_err(|_| invalid("invalid DAV CDATA"))?);
                }
            }
            Event::GeneralRef(reference) => {
                let reference = std::str::from_utf8(reference.as_ref())
                    .map_err(|_| invalid("invalid DAV XML entity"))?;
                let escaped = format!("&{reference};");
                let value = quick_xml::escape::unescape(&escaped)
                    .map_err(|_| invalid("unknown DAV XML entity"))?;
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                } else {
                    return Err(invalid("entity outside DAV root"));
                }
            }
            Event::DocType(_) => return Err(invalid("DAV XML document types are not allowed")),
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            _ => return Err(invalid("unsupported DAV XML event")),
        }
    }
    if !stack.is_empty() {
        return Err(invalid("truncated DAV XML"));
    }
    root.ok_or_else(|| invalid("missing DAV XML root"))
}
fn child<'a>(node: &'a XmlNode, name: &str) -> AppResult<Option<&'a XmlNode>> {
    let mut matches = node
        .children
        .iter()
        .filter(|child| child.dav && child.name == name);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(invalid("duplicate DAV lock property"));
    }
    Ok(first)
}
fn required<'a>(node: &'a XmlNode, name: &str) -> AppResult<&'a XmlNode> {
    child(node, name)?.ok_or_else(|| invalid("missing DAV lock property"))
}
fn collect<'a>(node: &'a XmlNode, name: &str, output: &mut Vec<&'a XmlNode>) {
    if node.dav && node.name == name {
        output.push(node);
    }
    for child in &node.children {
        collect(child, name, output);
    }
}
fn parse_grant(
    bytes: &[u8],
    header_token: Option<&str>,
    root: &Url,
    expected_token: Option<&str>,
) -> AppResult<(String, u64)> {
    let document = parse_xml(bytes)?;
    let mut locks = Vec::new();
    collect(&document, "activelock", &mut locks);
    if locks.len() != 1 {
        return Err(invalid("expected one active DAV lock"));
    }
    let lock = locks[0];
    let scope = required(lock, "lockscope")?;
    let lock_type = required(lock, "locktype")?;
    required(scope, "exclusive")?;
    required(lock_type, "write")?;
    if scope.children.iter().filter(|child| child.dav).count() != 1
        || lock_type.children.iter().filter(|child| child.dav).count() != 1
    {
        return Err(invalid("ambiguous lease scope or type"));
    }
    if required(lock, "depth")?.text.trim() != "infinity" {
        return Err(invalid("requires depth infinity collection lease"));
    }
    if let Some(lockroot) = child(lock, "lockroot")? {
        let href = required(lockroot, "href")?.text.trim();
        if href.is_empty() {
            return Err(invalid("missing lockroot href"));
        }
        let granted = root.join(href).map_err(|_| invalid("invalid lockroot"))?;
        let mut refreshed = granted.clone();
        if !refreshed.path().ends_with('/') {
            let path = format!("{}/", refreshed.path());
            refreshed.set_path(&path);
        }
        if &granted != root && !(expected_token.is_some() && &refreshed == root) {
            return Err(invalid("lease root does not match collection"));
        }
    }
    let token = required(required(lock, "locktoken")?, "href")?.text.trim();
    if !valid_token(token)
        || expected_token.map_or(header_token != Some(token), |expected| expected != token)
    {
        return Err(invalid("lease token mismatch"));
    }
    let timeout = required(lock, "timeout")?.text.trim();
    let seconds = timeout
        .strip_prefix("Second-")
        .filter(|seconds| !seconds.is_empty() && seconds.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|seconds| seconds.parse::<u64>().ok());
    if timeout == "Infinite" || seconds.is_some_and(|seconds| seconds > 300) {
        return Err(invalid("unsafe lease duration"));
    }
    let seconds = seconds
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| invalid("invalid lease timeout"))?;
    Ok((token.to_string(), seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }
    async fn server(
        responses: Vec<Vec<u8>>,
    ) -> (WebDavArchiveClient, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut last_put = Vec::new();
            for outgoing in responses {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0_u8; 8192];
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request
                        .windows(4)
                        .position(|part| part == b"\r\n\r\n")
                        .map(|index| index + 4)
                    {
                        let length = String::from_utf8_lossy(&request[..end])
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if request.len() >= end + length {
                            break;
                        }
                    }
                }
                if request.starts_with(b"PUT ") {
                    let end = request
                        .windows(4)
                        .position(|part| part == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    last_put = request[end..].to_vec();
                }
                let outgoing = if outgoing == b"ECHO_LAST_PUT" {
                    response("200 OK", "ETag: \"fence\"\r\n", &last_put)
                } else {
                    outgoing
                };
                requests.push(request);
                socket.write_all(&outgoing).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let api = WebDavArchiveClient::new(
            &client,
            &WebDavSyncConfig {
                server_url: format!("http://{address}/"),
                username: "fixture".into(),
                password: "fixture".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (api, task)
    }

    #[test]
    fn upgrade_permission_changes_with_credentials_but_history_scope_stays_the_same() {
        use super::super::archive::approval::{self, SyncProtocolUpgrade};
        let http = Client::builder().no_proxy().build().unwrap();
        let mut config = WebDavSyncConfig {
            server_url: format!("https://fixture.invalid/{}/", uuid::Uuid::new_v4()),
            username: "fixture-username".into(),
            password: "fixture-first-credential".into(),
            ..Default::default()
        };
        let original = WebDavArchiveClient::new(&http, &config).unwrap();
        let target = original.target();
        let scope = original.scope();
        assert!(!target.contains(&config.username));
        assert!(!target.contains(&config.password));
        let challenge = SyncProtocolUpgrade::new("webdav", target, b"backup", 0);
        approval::approve_verified(&challenge, &challenge).unwrap();
        config.password = "fixture-replaced-credential".into();
        let changed = WebDavArchiveClient::new(&http, &config).unwrap();
        assert_eq!(changed.scope(), scope);
        let replacement = SyncProtocolUpgrade::new("webdav", changed.target(), b"backup", 0);
        assert!(approval::require(&replacement).is_err());
        assert!(approval::approve_verified(&challenge, &replacement).is_err());
        approval::consume(&challenge).unwrap();
    }

    #[tokio::test]
    async fn initial_archive_upload_uses_create_only_for_every_object_and_manifest() {
        let prepared = archive::prepare(
            &super::super::models::SyncData::default(),
            Some("initial".into()),
        )
        .unwrap();
        let mut responses = vec![response("201 Created", "", b""); prepared.objects.len() + 1];
        responses.push(response("200 OK", "ETag: \"root\"\r\n", &prepared.content));
        let (api, task) = server(responses).await;
        api.put_archive(&prepared, None, &HashSet::new(), None)
            .await
            .unwrap();
        let requests = task.await.unwrap();
        for request in &requests[..requests.len() - 1] {
            let text = String::from_utf8_lossy(request).to_ascii_lowercase();
            assert!(text.starts_with("put "));
            assert!(text.contains("if-none-match: *\r\n"));
            assert!(!text.contains("if-match:"));
        }
    }

    #[tokio::test]
    async fn empty_target_is_validated_as_a_collection_through_depth_zero_propfind() {
        let document=b"<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";
        let (api, task) = server(vec![
            response("404 Not Found", "", b""),
            response("207 Multi-Status", "", document),
        ])
        .await;
        api.validate_connection().await.unwrap();
        let requests = task.await.unwrap();
        let probe = String::from_utf8_lossy(&requests[1]).to_ascii_lowercase();
        assert!(probe.starts_with("propfind / "));
        assert!(probe.contains("depth: 0"));
        assert!(validate_collection(
            &String::from_utf8_lossy(document)
                .replace("<d:collection/>", "")
                .into_bytes()
        )
        .is_err());
        assert!(validate_collection(
            &String::from_utf8_lossy(document)
                .replace("HTTP/1.1 200 OK", "HTTP/1.1 404 Not Found")
                .into_bytes()
        )
        .is_err());
        assert!(validate_collection(b"<d:multistatus xmlns:d=\"DAV:\"><d:response><d:status>HTTP/1.1 200 OK</d:status></d:response></d:multistatus>").is_err());
    }

    #[tokio::test]
    async fn stale_strong_etag_fails_without_an_unconditional_overwrite() {
        let prepared = archive::prepare(&super::super::models::SyncData::default(), None).unwrap();
        let (api, task) = server(vec![response("412 Precondition Failed", "", b"")]).await;
        let observed = RemoteFile {
            content: vec![1],
            fingerprint: "old".into(),
            etag: Some("\"old\"".into()),
        };
        let error = api
            .put_archive(
                &prepared,
                Some(&observed),
                &prepared.objects.keys().cloned().collect(),
                None,
            )
            .await
            .unwrap_err();
        assert!(is_conflict(&error));
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(String::from_utf8_lossy(&requests[0])
            .to_ascii_lowercase()
            .contains("if-match: \"old\""));
        let weak = RemoteFile {
            etag: None,
            ..observed
        };
        assert!(matches!(
            api.put_archive(&prepared, Some(&weak), &HashSet::new(), None)
                .await
                .unwrap_err(),
            AppError::Sync(SyncFailure::WebDavMissingCondition)
        ));
    }

    const COLLECTION: &str = "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";

    fn failure_kind(result: AppResult<()>) -> String {
        match result {
            Ok(()) => "ok".into(),
            Err(AppError::Sync(failure)) => failure.to_string(),
            Err(_) => "invalid".into(),
        }
    }

    #[test]
    fn directory_probe_classifies_the_android_failure_kinds() {
        let probe = |status: u16, body: &str| {
            failure_kind(directory_status(StatusCode::from_u16(status).unwrap(), body.as_bytes()))
        };
        assert_eq!(probe(207, COLLECTION), "ok");
        assert_eq!(probe(207, &COLLECTION.replace("<d:collection/>", "")), "WEBDAV_NOT_DIRECTORY");
        assert_eq!(
            probe(207, "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>/</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response></d:multistatus>"),
            "WEBDAV_DIRECTORY_NOT_FOUND"
        );
        let two_responses = COLLECTION.replace(
            "</d:multistatus>",
            "<d:response><d:href>/other</d:href></d:response></d:multistatus>",
        );
        assert_eq!(probe(207, &two_responses), "invalid");
        assert_eq!(probe(207, &COLLECTION.replace("200 OK", "404 Not Found")), "invalid");
        assert_eq!(probe(200, COLLECTION), "invalid", "a non-207 success is not a directory listing");
        assert_eq!(probe(401, ""), "WEBDAV_AUTH_FAILED");
        assert_eq!(probe(403, ""), "WEBDAV_ACCESS_DENIED");
        assert_eq!(probe(404, ""), "WEBDAV_DIRECTORY_NOT_FOUND");
        assert_eq!(failure_kind(Err(status_error(StatusCode::UNAUTHORIZED, "GET"))), "WEBDAV_AUTH_FAILED");
        assert_eq!(failure_kind(Err(status_error(StatusCode::FORBIDDEN, "GET"))), "WEBDAV_ACCESS_DENIED");
    }

    #[tokio::test]
    async fn a_missing_sync_folder_is_not_mistaken_for_an_empty_remote() {
        let (api, task) = server(vec![
            response("404 Not Found", "", b""),
            response("404 Not Found", "", b""),
        ])
        .await;
        let result = api
            .get(archive::MANIFEST_FILE, None, archive::MAX_OBJECT_BYTES)
            .await
            .map(|_| ());
        assert_eq!(failure_kind(result), "WEBDAV_DIRECTORY_NOT_FOUND");
        let requests = task.await.unwrap();
        let probe = String::from_utf8_lossy(&requests[1]).to_ascii_lowercase();
        assert!(probe.starts_with("propfind / "));
        assert!(probe.contains("depth: 0"));
    }

    #[tokio::test]
    async fn manifest_put_409_is_a_conflict_only_while_the_folder_exists() {
        let prepared = archive::prepare(&super::super::models::SyncData::default(), None).unwrap();
        let verified: HashSet<String> = prepared.objects.keys().cloned().collect();
        let (api, task) = server(vec![
            response("409 Conflict", "", b""),
            response("207 Multi-Status", "", COLLECTION.as_bytes()),
        ])
        .await;
        let error = api.put_archive(&prepared, None, &verified, None).await.unwrap_err();
        assert!(is_conflict(&error), "a 409 inside an existing folder is retried as a conflict");
        task.await.unwrap();

        let (api, task) = server(vec![
            response("409 Conflict", "", b""),
            response("404 Not Found", "", b""),
        ])
        .await;
        let result = api.put_archive(&prepared, None, &verified, None).await.map(|_| ());
        assert_eq!(failure_kind(result), "WEBDAV_DIRECTORY_NOT_FOUND");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn finite_lease_guards_weak_etag_publication_and_release() {
        let prepared = archive::prepare(&super::super::models::SyncData::default(), None).unwrap();
        let lock = grant("12345", "Second-300", None);
        let responses = vec![
            response("200 OK", "Lock-Token: 12345\r\n", lock.as_bytes()),
            response("200 OK", "", b"old"),
            response("204 No Content", "", b""),
            response("200 OK", "ETag: \"new\"\r\n", &prepared.content),
            response("204 No Content", "", b""),
        ];
        let (api, task) = server(responses).await;
        let lease = api.acquire_lease().await.unwrap().unwrap();
        let observed = RemoteFile {
            content: b"old".to_vec(),
            fingerprint: archive::digest(b"old"),
            etag: None,
        };
        api.put_archive(
            &prepared,
            Some(&observed),
            &prepared.objects.keys().cloned().collect(),
            Some(&lease),
        )
        .await
        .unwrap();
        lease.release().await.unwrap();
        let requests = task.await.unwrap();
        let put = String::from_utf8_lossy(&requests[2]).to_ascii_lowercase();
        assert!(put.starts_with("put "));
        assert!(put.contains("if: <http://"));
        assert!(put.contains("(<12345>)"));
        assert!(!put.contains("if-match:"));
        assert!(String::from_utf8_lossy(&requests[4]).starts_with("UNLOCK "));
    }

    #[tokio::test]
    async fn failed_lease_refresh_stops_before_put_and_still_releases() {
        let (api, task) = server(vec![
            response("500 Internal Server Error", "", b""),
            response("204 No Content", "", b""),
        ])
        .await;
        let lease = Lease {
            client: api.clone(),
            state: Arc::new(tokio::sync::Mutex::new(LeaseState {
                token: "12345".into(),
                expires: Instant::now() + Duration::from_secs(30),
                closed: false,
            })),
        };
        let result = api
            .execute(
                Method::PUT,
                api.file_url(archive::MANIFEST_FILE).unwrap(),
                Some(&lease),
                Some(("If-None-Match", "*")),
                Some(b"body"),
                64 * 1024,
            )
            .await;
        assert!(result.is_err());
        lease.release().await.unwrap();
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(String::from_utf8_lossy(&requests[0]).starts_with("LOCK "));
        assert!(String::from_utf8_lossy(&requests[1]).starts_with("UNLOCK "));
    }

    #[tokio::test]
    async fn uncertain_unlock_requires_reacquisition_matching_manifest_and_complete_closure() {
        let prepared = archive::prepare(&super::super::models::SyncData::default(), None).unwrap();
        let lock = grant("12345", "Second-300", None);
        let mut responses = vec![
            response("200 OK", "Lock-Token: <12345>\r\n", lock.as_bytes()),
            response("200 OK", "ETag: \"published\"\r\n", &prepared.content),
        ];
        // recovery follows Merkle traversal, so objects must be served by requested path
        let (protocol, raw) = archive::test_manifest(&prepared.content);
        assert_eq!(protocol, 4);
        let order = archive::test_object_paths(&raw);
        for path in order {
            responses.push(response("200 OK", "", &prepared.objects[&path]));
        }
        responses.push(response("204 No Content", "", b""));
        let (api, task) = server(responses).await;
        let published = RemoteFile {
            content: prepared.content.clone(),
            fingerprint: archive::digest(&prepared.content),
            etag: Some("\"published\"".into()),
        };
        api.verify_release_recovery(&published, &prepared.objects.keys().cloned().collect())
            .await
            .unwrap();
        let requests = task.await.unwrap();
        assert!(String::from_utf8_lossy(requests.last().unwrap()).starts_with("UNLOCK "));
        assert!(is_recoverable_release_failure(&status_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNLOCK"
        )));
        assert!(!is_recoverable_release_failure(&status_error(
            StatusCode::CONFLICT,
            "UNLOCK"
        )));
    }

    fn listing_resource(href: &str, etag: Option<&str>, collection: bool) -> String {
        format!("<d:response><d:href>{href}</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop><d:resourcetype>{}</d:resourcetype>{}</d:prop></d:propstat></d:response>",if collection{"<d:collection/>"}else{""},etag.map(|etag|format!("<d:getetag>{etag}</d:getetag>")).unwrap_or_default())
    }

    #[tokio::test]
    async fn retired_object_is_deleted_only_after_fenced_closure_and_strong_object_condition() {
        let prepared = archive::prepare(
            &super::super::models::SyncData::default(),
            Some("observed".into()),
        )
        .unwrap();
        let retired = format!("neriplayer-sync-v4-{}.zst", "a".repeat(64));
        let mut listing = format!(
            "<d:multistatus xmlns:d=\"DAV:\">{}",
            listing_resource("/", None, true)
        );
        for path in prepared.objects.keys() {
            listing.push_str(&listing_resource(
                &format!("/{path}"),
                Some("\"protected\""),
                false,
            ));
        }
        listing.push_str(&listing_resource(
            &format!("/{retired}"),
            Some("\"retired\""),
            false,
        ));
        listing.push_str(&listing_resource(
            "/unrelated.txt",
            Some("\"unrelated\""),
            false,
        ));
        listing.push_str("</d:multistatus>");
        let mut responses = vec![
            response("207 Multi-Status", "", listing.as_bytes()),
            response("200 OK", "ETag: \"observed\"\r\n", &prepared.content),
            response("204 No Content", "", b""),
            b"ECHO_LAST_PUT".to_vec(),
        ];
        let (_, raw) = archive::test_manifest(&prepared.content);
        for path in archive::test_object_paths(&raw) {
            responses.push(response("200 OK", "", &prepared.objects[&path]));
        }
        responses.extend([
            b"ECHO_LAST_PUT".to_vec(),
            response("204 No Content", "", b""),
            b"ECHO_LAST_PUT".to_vec(),
        ]);
        let (api, task) = server(responses).await;
        let protected = prepared.objects.keys().cloned().collect();
        let journal = super::super::webdav_gc::Journal::default().observe(
            &std::collections::BTreeMap::from([(retired.clone(), "\"retired\"".into())]),
            &HashSet::new(),
            1000,
            10000,
        );
        let path = gc_journal_path(&api.scope());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        let lease = Lease {
            client: api.clone(),
            state: Arc::new(tokio::sync::Mutex::new(LeaseState {
                token: "12345".into(),
                expires: Instant::now() + Duration::from_secs(300),
                closed: false,
            })),
        };
        let published = RemoteFile {
            content: prepared.content.clone(),
            fingerprint: archive::digest(&prepared.content),
            etag: Some("\"observed\"".into()),
        };
        let current = api
            .maintain_gc_at(
                &published,
                &protected,
                &HashSet::new(),
                Some(&lease),
                1000 + super::super::webdav_gc::GRACE_MS,
                10000 + super::super::webdav_gc::GRACE_MS,
            )
            .await
            .unwrap();
        assert_ne!(current.fingerprint, published.fingerprint);
        assert!(archive::same_closure(&current.content, &published.content).unwrap());
        lease.state.lock().await.closed = true;
        let requests = task.await.unwrap();
        let deletes = requests
            .iter()
            .filter(|request| request.starts_with(b"DELETE "))
            .collect::<Vec<_>>();
        assert_eq!(deletes.len(), 1);
        let deletion = String::from_utf8_lossy(deletes[0]).to_ascii_lowercase();
        assert!(deletion.contains(&retired));
        assert!(deletion.contains("if-match: \"retired\""));
        assert!(deletion.contains("(<12345>)"));
        assert!(!deletion.contains("unrelated"));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn objects_of_the_replaced_manifest_restart_their_retention() {
        let prepared = archive::prepare(
            &super::super::models::SyncData::default(),
            Some("observed".into()),
        )
        .unwrap();
        let replaced = format!("neriplayer-sync-v4-{}.zst", "b".repeat(64));
        let mut listing = format!(
            "<d:multistatus xmlns:d=\"DAV:\">{}",
            listing_resource("/", None, true)
        );
        for path in prepared.objects.keys() {
            listing.push_str(&listing_resource(&format!("/{path}"), Some("\"protected\""), false));
        }
        listing.push_str(&listing_resource(&format!("/{replaced}"), Some("\"replaced\""), false));
        listing.push_str("</d:multistatus>");
        let (api, task) = server(vec![
            response("207 Multi-Status", "", listing.as_bytes()),
            response("200 OK", "ETag: \"observed\"\r\n", &prepared.content),
        ])
        .await;
        // 这个对象早已过了观察期，只是刚被别的设备的旧清单重新引用过
        let journal = super::super::webdav_gc::Journal::default().observe(
            &std::collections::BTreeMap::from([(replaced.clone(), "\"replaced\"".into())]),
            &HashSet::new(),
            1000,
            10000,
        );
        let path = gc_journal_path(&api.scope());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        let lease = Lease {
            client: api.clone(),
            state: Arc::new(tokio::sync::Mutex::new(LeaseState {
                token: "12345".into(),
                expires: Instant::now() + Duration::from_secs(300),
                closed: false,
            })),
        };
        let published = RemoteFile {
            content: prepared.content.clone(),
            fingerprint: archive::digest(&prepared.content),
            etag: Some("\"observed\"".into()),
        };
        let current = api
            .maintain_gc_at(
                &published,
                &prepared.objects.keys().cloned().collect(),
                &HashSet::from([replaced.clone()]),
                Some(&lease),
                1000 + super::super::webdav_gc::GRACE_MS,
                10000 + super::super::webdav_gc::GRACE_MS,
            )
            .await
            .unwrap();
        assert_eq!(current.fingerprint, published.fingerprint, "nothing was eligible, so no fence was published");
        lease.state.lock().await.closed = true;
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| !request.starts_with(b"DELETE ")));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn xml_references_and_listing_duplicates_are_checked_without_external_entities() {
        let root = Url::parse("https://example.test/?a=1&b=2").unwrap();
        let xml = grant(
            "12345",
            "Second-300",
            Some("https://example.test/?a=1&amp;b=2"),
        );
        assert!(parse_grant(xml.as_bytes(), Some("12345"), &root, None).is_ok());
        assert!(parse_xml(b"<d:prop xmlns:d=\"DAV:\">&#x41;&amp;&lt;</d:prop>").is_ok());
        assert!(parse_xml(b"<!DOCTYPE d:prop [<!ENTITY secret SYSTEM 'file:///private'>]><d:prop xmlns:d=\"DAV:\">&secret;</d:prop>").is_err());
        let collection = listing_resource("/", None, true);
        let duplicate =
            format!("<d:multistatus xmlns:d=\"DAV:\">{collection}{collection}</d:multistatus>");
        assert!(parse_listing(
            duplicate.as_bytes(),
            &Url::parse("https://example.test/").unwrap()
        )
        .is_err());
    }
    fn grant(token: &str, timeout: &str, root: Option<&str>) -> String {
        format!("<d:prop xmlns:d=\"DAV:\"><d:lockdiscovery><d:activelock><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:depth>infinity</d:depth><d:timeout>{timeout}</d:timeout><d:locktoken><d:href>{token}</d:href></d:locktoken>{}</d:activelock></d:lockdiscovery></d:prop>",root.map(|root|format!("<d:lockroot><d:href>{root}</d:href></d:lockroot>")).unwrap_or_default())
    }
    #[test]
    fn validates_real_provider_token_forms_roots_namespaces_and_finite_timeouts() {
        let root = Url::parse("https://example.test/%E4%B8%AD%E6%96%87/").unwrap();
        for token in ["opaquelocktoken:uuid", "12345"] {
            let header = parse_header_token(token).unwrap();
            assert_eq!(
                parse_grant(
                    grant(token, "Second-300", None).as_bytes(),
                    Some(&header),
                    &root,
                    None
                )
                .unwrap()
                .0,
                token
            );
            assert!(parse_grant(
                grant(token, "Infinite", None).as_bytes(),
                Some(&header),
                &root,
                None
            )
            .is_err());
            assert!(parse_grant(
                grant(token, "Second-301", None).as_bytes(),
                Some(&header),
                &root,
                None
            )
            .is_err());
            assert!(parse_grant(
                grant(token, "Second-300", Some("/other/")).as_bytes(),
                Some(&header),
                &root,
                None
            )
            .is_err());
        }
        assert!(parse_header_token("relative-token").is_none());
        assert!(parse_header_token("<12345>").is_some());
        let duplicate = grant("12345", "Second-300", None).replace(
            "</d:activelock>",
            "<d:depth>infinity</d:depth></d:activelock>",
        );
        assert!(parse_grant(duplicate.as_bytes(), Some("12345"), &root, None).is_err());
        let wrong_namespace = grant("12345", "Second-300", None).replace("DAV:", "evil:");
        assert!(parse_grant(wrong_namespace.as_bytes(), Some("12345"), &root, None).is_err());
        assert!(!strong_etag("W/\"weak\""));
        assert!(!strong_etag("unquoted"));
        assert!(strong_etag("\"strong\""));
    }
    #[test]
    fn preserves_query_routing_encodes_paths_and_excludes_fragment_from_identity() {
        let config = WebDavSyncConfig {
            server_url: "https://example.test/base/?route=one#fragment".into(),
            base_path: "中文 目录".into(),
            ..Default::default()
        };
        let api = WebDavArchiveClient::new(&Client::new(), &config).unwrap();
        assert_eq!(api.file_url(archive::MANIFEST_FILE).unwrap().as_str(),"https://example.test/base/%E4%B8%AD%E6%96%87%20%E7%9B%AE%E5%BD%95/neriplayer-sync-v3.manifest?route=one");
        assert!(api.file_url("../escape").is_err());
    }
}

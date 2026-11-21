// WebDAV API 客户端
use reqwest::Client;
use sha2::{Digest, Sha256};

/// WebDAV 响应正文上限，取序列化层两档限制中较大的一档
///
/// 具体格式的精确上限由 `serializer::ensure_remote_content_size` 判定；
/// 这里只负责不让一个超大响应在到达那一步之前就把内存吃满。
const MAX_REMOTE_BODY_BYTES: u64 = 12 * 1024 * 1024;
use neri_common::error::{AppError, AppResult};

pub struct WebDavApiClient {
    http: Client,
    server_url: String,
    username: String,
    password: String,
    base_path: String,
}

const SYNC_FILENAME: &str = "neriplayer-sync.json";

/// 412 并发冲突的可识别错误文案前缀
///
/// AppError 不归本模块管（新增变体会波及全局 IPC 序列化），
/// 用固定前缀 + 判别函数让 manager 能把 412 与其他失败区分开做重试。
const PRECONDITION_CONFLICT_PREFIX: &str = "WebDAV precondition failed (412)";

/// 判断错误是否为 If-Match 未命中导致的 412 并发冲突
pub fn is_precondition_conflict(error: &neri_common::error::AppError) -> bool {
    matches!(error, AppError::Api(message) if message.starts_with(PRECONDITION_CONFLICT_PREFIX))
}

impl WebDavApiClient {
    pub fn new(
        http: &Client,
        server_url: &str,
        username: &str,
        password: &str,
        base_path: &str,
    ) -> Self {
        Self {
            http: http.clone(),
            server_url: server_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            base_path: base_path.to_string(),
        }
    }

    fn remote_url(&self) -> String {
        if self.base_path.is_empty() {
            format!("{}/{}", self.server_url, SYNC_FILENAME)
        } else {
            let bp = self.base_path.trim_matches('/');
            format!("{}/{}/{}", self.server_url, bp, SYNC_FILENAME)
        }
    }

    /// 指纹算在**原始字节**上
    ///
    /// 必须与 Android 的 `calculateFingerprint(ByteArray)` 一致：
    /// 一端算字符串、一端算字节，同一份内容会得出不同指纹，
    /// 双端就会各自认为「远端变了」而无限对传。
    fn sha256_fingerprint(content: &[u8]) -> String {
        let hash = Sha256::digest(content);
        hex::encode(hash)
    }

    /// 验证连接（GET 请求，200/404 均视为连接成功）
    pub async fn validate_connection(&self) -> AppResult<()> {
        let url = self.remote_url();
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;

        let status = resp.status().as_u16();
        match status {
            200 | 404 => Ok(()),
            401 | 403 => Err(AppError::Api("WebDAV authentication failed".into())),
            _ => Err(AppError::Api(format!(
                "WebDAV connection failed ({})",
                status
            ))),
        }
    }

    /// 获取文件内容、指纹与 ETag（服务端未返回 ETag 时为 None）
    /// 不存在时返回 Ok(None)
    pub async fn get_file_content(&self) -> AppResult<Option<(Vec<u8>, String, Option<String>)>> {
        let url = self.remote_url();
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if status == 401 || status == 403 {
            return Err(AppError::Api("WebDAV authentication failed".into()));
        }
        if !resp.status().is_success() {
            return Err(AppError::Api(format!("WebDAV GET failed ({})", status)));
        }

        // 捕获 ETag 供后续 PUT 带 If-Match，弱 ETag（W/ 前缀）不可用于写前提条件
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.starts_with("W/"))
            .map(String::from);

        // 服务端声明的体积就已超限时直接拒收，不必先把整个响应读进内存
        if let Some(declared) = resp.content_length() {
            if declared > MAX_REMOTE_BODY_BYTES {
                return Err(AppError::Api(format!(
                    "WebDAV backup is too large: {declared} bytes"
                )));
            }
        }

        // 按字节读：省流备份是 GZIP 二进制，走 text() 会被 UTF-8 转换毁掉
        let content = resp.bytes().await?.to_vec();
        // Content-Length 可缺失或撒谎，落地后再判一次
        if content.len() as u64 > MAX_REMOTE_BODY_BYTES {
            return Err(AppError::Api(format!(
                "WebDAV backup is too large: {} bytes",
                content.len()
            )));
        }
        let fingerprint = Self::sha256_fingerprint(&content);
        Ok(Some((content, fingerprint, etag)))
    }

    /// 上传文件内容，返回 SHA-256 指纹
    ///
    /// 省流模式以 octet-stream 传原始 GZIP 字节，与 Android 对齐；
    /// 对二进制正文声明 `application/json` 会让部分服务端做转码甚至拒收。
    ///
    /// `if_match` 为 GET 时捕获的 ETag：带上后 GET→PUT 窗口内他端的写入会让
    /// 服务端返回 412 而不是被本次 PUT 静默覆盖（对齐 Android WebDavApiClient
    /// 的 If-Match）；无 ETag 时退化为现状的无条件 PUT。
    pub async fn update_file_content(
        &self,
        content: &[u8],
        data_saver: bool,
        if_match: Option<&str>,
    ) -> AppResult<String> {
        let url = self.remote_url();
        // 省流传 GZIP 二进制，非省流传 JSON 文本；与 Android 的 mediaType 选择一致
        let media_type = if data_saver {
            "application/octet-stream"
        } else {
            "application/json; charset=utf-8"
        };
        let mut request = self
            .http
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", media_type);
        if let Some(etag) = if_match {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        let resp = request.body(content.to_vec()).send().await?;

        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            return Err(AppError::Api("WebDAV authentication failed".into()));
        }
        if status == 412 {
            return Err(AppError::Api(format!(
                "{}: remote file changed since last read",
                PRECONDITION_CONFLICT_PREFIX
            )));
        }
        // WebDAV PUT 成功通常返回 200/201/204
        if !resp.status().is_success() {
            return Err(AppError::Api(format!("WebDAV PUT failed ({})", status)));
        }

        Ok(Self::sha256_fingerprint(content))
    }
}

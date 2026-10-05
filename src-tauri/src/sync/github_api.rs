// GitHub Contents API 客户端
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::{Client, RequestBuilder, StatusCode};

use crate::error::AppError;

const GITHUB_API_BASE: &str = "https://api.github.com";
const MAX_SYNC_FILE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_API_ERROR_CHARS: usize = 240;

pub type GitHubResult<T> = Result<T, GitHubApiError>;

#[derive(Debug, thiserror::Error)]
pub enum GitHubApiError {
    #[error("GitHub token expired or invalid")]
    TokenExpired,
    #[error("GitHub resource not found: {0}")]
    NotFound(String),
    #[error("GitHub content conflict ({status}): {message}")]
    ContentConflict { status: u16, message: String },
    #[error("GitHub API request failed ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Invalid GitHub API response: {0}")]
    InvalidResponse(String),
    #[error("GitHub network error: {0}")]
    Network(reqwest::Error),
}

impl From<reqwest::Error> for GitHubApiError {
    fn from(error:reqwest::Error)->Self {Self::Network(error.without_url())}
}

impl GitHubApiError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }

    pub fn is_content_conflict(&self) -> bool {
        matches!(self, Self::ContentConflict { .. })
    }
}

impl From<GitHubApiError> for AppError {
    fn from(error: GitHubApiError) -> Self {
        match error {
            GitHubApiError::TokenExpired => AppError::Api("GitHub token expired or invalid".into()),
            GitHubApiError::NotFound(message) => AppError::NotFound(message),
            GitHubApiError::ContentConflict { status, message } => {
                AppError::Api(format!("GitHub content conflict ({}): {}", status, message))
            }
            GitHubApiError::Api { status, message } => AppError::Api(format!(
                "GitHub API request failed ({}): {}",
                status, message
            )),
            GitHubApiError::InvalidResponse(message) => AppError::Other(message),
            GitHubApiError::Network(error) => AppError::Network(error),
        }
    }
}

pub struct GitHubApiClient {
    http: Client,
    token: String,
    api_base: String,
}

#[derive(Clone, Debug)]
pub struct GitHubHead {
    pub branch: String,
    pub sha: String,
    pub repository_id: String,
}

impl GitHubApiClient {
    async fn bounded_body(mut response: reqwest::Response, maximum: usize) -> GitHubResult<Vec<u8>> {
        if response.content_length().is_some_and(|bytes| bytes > maximum as u64) {
            return Err(GitHubApiError::InvalidResponse("response exceeds sync budget".into()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > maximum.saturating_sub(body.len()) {
                return Err(GitHubApiError::InvalidResponse("response exceeds sync budget".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn archive_json(&self, request: RequestBuilder) -> GitHubResult<serde_json::Value> {
        let response = self.request(request).send().await?;
        let status = response.status();
        let body = Self::bounded_body(response, 4 * 1024 * 1024).await?;
        if status == StatusCode::UNAUTHORIZED { return Err(GitHubApiError::TokenExpired); }
        if !status.is_success() { return Err(api_error(status, String::from_utf8_lossy(&body).into(), "sync archive", true)); }
        serde_json::from_slice(&body).map_err(|error| GitHubApiError::InvalidResponse(format!("invalid archive JSON: {error}")))
    }

    pub async fn get_repository_head(&self, owner: &str, repo: &str) -> GitHubResult<GitHubHead> {
        let repository = self.archive_json(self.http.get(self.endpoint(&format!("repos/{owner}/{repo}")))).await?;
        let branch = required_json_string(&repository,"default_branch")?;
        let repository_id = required_json_string(&repository,"node_id")?;
        let reference = self.archive_json(self.http.get(self.endpoint(&format!("repos/{owner}/{repo}/git/ref/heads/{}",urlencoding::encode(&branch))))).await?;
        let sha = required_json_string(&reference["object"],"sha")?;
        Ok(GitHubHead {branch,sha,repository_id})
    }

    pub async fn get_file_at_ref(&self, owner: &str, repo: &str, path: &str, reference: &str, maximum: usize) -> GitHubResult<Option<Vec<u8>>> {
        if path.contains('/') || path.contains('\\') || path.is_empty() || reference.is_empty() { return Err(GitHubApiError::InvalidResponse("invalid archive path or fixed reference".into())); }
        let response = self.http.get(self.endpoint(&format!("repos/{owner}/{repo}/contents/{}",urlencoding::encode(path))))
            .query(&[("ref",reference)])
            .bearer_auth(&self.token)
            .header("Accept","application/vnd.github.raw+json")
            .header("X-GitHub-Api-Version","2022-11-28")
            .send().await?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND { return Ok(None); }
        let body = Self::bounded_body(response,maximum).await?;
        if status == StatusCode::UNAUTHORIZED { return Err(GitHubApiError::TokenExpired); }
        if !status.is_success() { return Err(api_error(status,String::from_utf8_lossy(&body).into(),"read fixed archive",false)); }
        Ok(Some(body))
    }

    pub async fn publish_archive(&self, owner: &str, repo: &str, head: &GitHubHead, prepared: &super::archive::PreparedArchive, verified_paths: &std::collections::HashSet<String>) -> GitHubResult<String> {
        use super::archive::{MANIFEST_FILE, canonical_object_path};
        let commit = self.archive_json(self.http.get(self.endpoint(&format!("repos/{owner}/{repo}/git/commits/{}",head.sha)))).await?;
        let base_tree = required_json_string(&commit["tree"],"sha")?;
        let listing = self.archive_json(self.http.get(self.endpoint(&format!("repos/{owner}/{repo}/git/trees/{base_tree}")))).await?;
        if listing["truncated"].as_bool()!=Some(false) { return Err(GitHubApiError::InvalidResponse("archive tree listing is truncated or incomplete".into())); }
        let entries = listing["tree"].as_array().ok_or_else(||GitHubApiError::InvalidResponse("missing archive tree entries".into()))?;
        let mut existing = std::collections::HashMap::new();
        for entry in entries {
            let path = required_json_string(entry,"path")?;
            if path==MANIFEST_FILE && (entry["type"].as_str()!=Some("blob") || entry["mode"].as_str()!=Some("100644")) {return Err(GitHubApiError::InvalidResponse("manifest is not a regular file".into()));}
            if canonical_object_path(&path) {
                if entry["type"].as_str()!=Some("blob") || entry["mode"].as_str()!=Some("100644") { return Err(GitHubApiError::InvalidResponse("owned archive path is not a regular file".into())); }
                existing.insert(path,required_json_string(entry,"sha")?);
            }
        }
        let mut changes = Vec::new();
        for (path,content) in prepared.objects.iter().chain(std::iter::once((&MANIFEST_FILE.to_string(),&prepared.content))) {
            if path!=MANIFEST_FILE && !canonical_object_path(path) {return Err(GitHubApiError::InvalidResponse("object outside archive closure".into()));}
            if path!=MANIFEST_FILE && verified_paths.contains(path) && existing.contains_key(path) {continue;}
            let blob = self.archive_json(self.http.post(self.endpoint(&format!("repos/{owner}/{repo}/git/blobs"))).json(&serde_json::json!({"content":BASE64.encode(content),"encoding":"base64"}))).await?;
            changes.push(serde_json::json!({"path":path,"mode":"100644","type":"blob","sha":required_json_string(&blob,"sha")?}));
        }
        for path in existing.keys().filter(|path|!prepared.objects.contains_key(*path)) {
            changes.push(serde_json::json!({"path":path,"mode":"100644","type":"blob","sha":null}));
        }
        let tree = self.archive_json(self.http.post(self.endpoint(&format!("repos/{owner}/{repo}/git/trees"))).json(&serde_json::json!({"base_tree":base_tree,"tree":changes}))).await?;
        let commit = self.archive_json(self.http.post(self.endpoint(&format!("repos/{owner}/{repo}/git/commits"))).json(&serde_json::json!({"message":"Sync from NeriPlayer Desktop","tree":required_json_string(&tree,"sha")?,"parents":[head.sha]}))).await?;
        let commit_sha = required_json_string(&commit,"sha")?;
        let mut endpoint = url::Url::parse(&self.api_base).map_err(|_|GitHubApiError::InvalidResponse("invalid GitHub API base".into()))?;
        endpoint.set_path(if endpoint.path().trim_end_matches('/')=="/api/v3" {"/api/graphql"} else if endpoint.path().trim_matches('/').is_empty() {"/graphql"} else {return Err(GitHubApiError::InvalidResponse("unsupported atomic GraphQL endpoint".into()));});
        endpoint.set_query(None); endpoint.set_fragment(None);
        let response = self.http.post(endpoint.clone()).bearer_auth(&self.token).header("Accept","application/json").json(&serde_json::json!({
            "query":"mutation NeriPlayerSyncPublish($input: UpdateRefsInput!) { updateRefs(input: $input) { clientMutationId } }",
            "variables":{"input":{"repositoryId":head.repository_id,"refUpdates":[{"name":format!("refs/heads/{}",head.branch),"beforeOid":head.sha,"afterOid":commit_sha,"force":false}],"clientMutationId":commit_sha}}
        })).send().await?;
        if response.url()!=&endpoint {return Err(GitHubApiError::InvalidResponse("atomic publication response redirected".into()));}
        let status=response.status(); let body=Self::bounded_body(response,64*1024).await?;
        if status==StatusCode::UNAUTHORIZED {return Err(GitHubApiError::TokenExpired);}
        if !status.is_success() {return Err(api_error(status,String::from_utf8_lossy(&body).into(),"atomic archive publication",true));}
        let result:serde_json::Value=serde_json::from_slice(&body).map_err(|_|GitHubApiError::InvalidResponse("invalid GraphQL publication response".into()))?;
        if let Some(errors)=result.get("errors") {
            let errors=errors.as_array().ok_or_else(||GitHubApiError::InvalidResponse("invalid GraphQL errors".into()))?;
            if !errors.is_empty() {
                let stale=errors.iter().any(|error|error["type"].as_str()==Some("STALE_DATA") || error["extensions"]["code"].as_str()==Some("STALE_DATA"));
                if stale {return Err(GitHubApiError::ContentConflict{status:409,message:"branch changed before atomic publication".into()});}
                return Err(GitHubApiError::Api{status:status.as_u16(),message:"GitHub atomic archive publication failed".into()});
            }
        }
        if result["data"]["updateRefs"]["clientMutationId"].as_str()!=Some(&commit_sha) {return Err(GitHubApiError::InvalidResponse("atomic publication acknowledgement mismatch".into()));}
        Ok(commit_sha)
    }
    pub fn new(http: &Client, token: &str) -> Self {
        Self::with_api_base(http, token, GITHUB_API_BASE)
    }

    fn with_api_base(http: &Client, token: &str, api_base: &str) -> Self {
        Self {
            http: http.clone(),
            token: token.to_string(),
            api_base: api_base.trim_end_matches('/').to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_api_base(http: &Client, token: &str, api_base: &str) -> Self {
        Self::with_api_base(http, token, api_base)
    }

    fn request(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.api_base, path.trim_start_matches('/'))
    }

    /// 验证 token，返回用户名
    pub async fn validate_token(&self) -> GitHubResult<String> {
        let response = self
            .request(self.http.get(self.endpoint("user")))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if !status.is_success() {
            return Err(api_error(status, body, "validate token", false));
        }

        let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            GitHubApiError::InvalidResponse(format!("failed to parse GitHub user: {}", error))
        })?;
        value["login"]
            .as_str()
            .filter(|login| !login.is_empty())
            .map(String::from)
            .ok_or_else(|| GitHubApiError::InvalidResponse("missing GitHub username".into()))
    }

    /// 创建私有仓库
    pub async fn create_repository(&self, repo_name: &str) -> GitHubResult<()> {
        let body = serde_json::json!({
            "name": repo_name,
            "private": true,
            "auto_init": true,
            "description": "NeriPlayer backup data"
        });
        let response = self
            .request(self.http.post(self.endpoint("user/repos")))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let response_body = response.text().await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if status.is_success()
            || (status == StatusCode::UNPROCESSABLE_ENTITY
                && response_body.contains("already exists"))
        {
            return Ok(());
        }
        Err(api_error(status, response_body, "create repository", false))
    }

    /// 检查仓库是否存在，返回默认分支名
    pub async fn check_repository(&self, owner: &str, repo: &str) -> GitHubResult<String> {
        let response = self
            .request(
                self.http
                    .get(self.endpoint(&format!("repos/{}/{}", owner, repo))),
            )
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if status == StatusCode::NOT_FOUND {
            return Err(GitHubApiError::NotFound(format!("{}/{}", owner, repo)));
        }
        if !status.is_success() {
            return Err(api_error(status, body, "check repository", false));
        }

        let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            GitHubApiError::InvalidResponse(format!("failed to parse GitHub repository: {}", error))
        })?;
        Ok(value["default_branch"]
            .as_str()
            .filter(|branch| !branch.is_empty())
            .unwrap_or("main")
            .to_string())
    }

    /// 获取文件内容和 SHA，只有 404 会返回 None
    /// 读取文件正文，返回原始字节
    ///
    /// 备份可能是原始 GZIP（新格式）也可能是文本（旧格式），
    /// 一律按字节返回交给序列化层判别；这里做 UTF-8 转换会把 GZIP 直接判死。
    pub async fn get_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
    ) -> GitHubResult<Option<(Vec<u8>, String)>> {
        let response = self
            .request(
                self.http
                    .get(self.endpoint(&format!("repos/{}/{}/contents/{}", owner, repo, path))),
            )
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if !status.is_success() {
            return Err(api_error(status, body, "get file", false));
        }

        let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            GitHubApiError::InvalidResponse(format!(
                "failed to parse GitHub file response: {}",
                error
            ))
        })?;
        let size = value["size"].as_u64().unwrap_or(0);
        if size > MAX_SYNC_FILE_BYTES {
            return Err(GitHubApiError::InvalidResponse(
                "remote backup file is too large".into(),
            ));
        }
        let content_b64 = value["content"]
            .as_str()
            .ok_or_else(|| GitHubApiError::InvalidResponse("missing file content".into()))?;
        let sha = value["sha"]
            .as_str()
            .filter(|sha| !sha.is_empty())
            .ok_or_else(|| GitHubApiError::InvalidResponse("missing file SHA".into()))?
            .to_string();

        let cleaned: String = content_b64
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();

        // Contents API 对超过 1MB 的文件只回 size, content 为空串且 encoding=none
        // 此时必须改走 raw 媒体类型, 否则会被误判成"文件损坏"
        if cleaned.is_empty() && size > 0 {
            log::info!(
                target: "sync",
                "GitHub contents API omitted inline content (size={}), falling back to raw",
                size,
            );
            let content = self.get_file_raw(owner, repo, path).await?;
            return Ok(Some((content, sha)));
        }

        // Contents API 的 content 字段永远是 Base64，解一次就得到文件原始字节
        let decoded = BASE64.decode(&cleaned).map_err(|error| {
            GitHubApiError::InvalidResponse(format!("failed to decode file content: {}", error))
        })?;
        if decoded.len() as u64 > MAX_SYNC_FILE_BYTES {
            return Err(GitHubApiError::InvalidResponse(
                "remote backup file is too large".into(),
            ));
        }

        Ok(Some((decoded, sha)))
    }

    /// 以 raw 媒体类型读取文件正文, 绕开 Contents API 的 1MB 内联上限
    async fn get_file_raw(&self, owner: &str, repo: &str, path: &str) -> GitHubResult<Vec<u8>> {
        // 不能复用 request(): reqwest 的 header() 是追加语义,
        // 追加第二个 Accept 会让 GitHub 仍按 JSON 返回
        let response = self
            .http
            .get(self.endpoint(&format!("repos/{}/{}/contents/{}", owner, repo, path)))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github.raw")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if status == StatusCode::NOT_FOUND {
            return Err(GitHubApiError::NotFound(path.to_string()));
        }
        if !status.is_success() {
            let body = response.text().await?;
            return Err(api_error(status, body, "get raw file", false));
        }

        let bytes = response.bytes().await?;
        if bytes.len() as u64 > MAX_SYNC_FILE_BYTES {
            return Err(GitHubApiError::InvalidResponse(
                "remote backup file is too large".into(),
            ));
        }
        Ok(bytes.to_vec())
    }

    /// 创建或更新文件，sha 为空时表示新建
    ///
    /// `content` 是文件的原始字节。Contents API 只收 Base64，
    /// 所以这里编码**一次**——省流备份本身已是 GZIP 二进制，
    /// 再叠一层 Base64 就是白白多出 1/3 流量。
    pub async fn update_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        content: &[u8],
        sha: &str,
        message: &str,
    ) -> GitHubResult<String> {
        let branch = self.check_repository(owner, repo).await?;
        let mut body = serde_json::json!({
            "message": message,
            "content": BASE64.encode(content),
            "branch": branch,
        });
        if !sha.is_empty() {
            body["sha"] = serde_json::Value::String(sha.to_string());
        }

        let response = self
            .request(
                self.http
                    .put(self.endpoint(&format!("repos/{}/{}/contents/{}", owner, repo, path))),
            )
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let response_body = response.text().await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::TokenExpired);
        }
        if !status.is_success() {
            return Err(api_error(status, response_body, "update file", true));
        }

        let value: serde_json::Value = serde_json::from_str(&response_body).map_err(|error| {
            GitHubApiError::InvalidResponse(format!(
                "failed to parse GitHub update response: {}",
                error
            ))
        })?;
        value["content"]["sha"]
            .as_str()
            .filter(|sha| !sha.is_empty())
            .map(String::from)
            .ok_or_else(|| GitHubApiError::InvalidResponse("missing updated file SHA".into()))
    }
}

fn required_json_string(value:&serde_json::Value,key:&str)->GitHubResult<String> {
    value.get(key).and_then(serde_json::Value::as_str).filter(|text|!text.is_empty()).map(String::from).ok_or_else(||GitHubApiError::InvalidResponse(format!("missing {key}")))
}

fn api_error(
    status: StatusCode,
    body: String,
    operation: &str,
    detect_content_conflict: bool,
) -> GitHubApiError {
    let message = safe_api_error_message(status, &body, operation);
    let status = status.as_u16();
    if detect_content_conflict
        && (status == 409 || (status == 422 && message.to_ascii_lowercase().contains("sha")))
    {
        GitHubApiError::ContentConflict { status, message }
    } else {
        GitHubApiError::Api { status, message }
    }
}

fn safe_api_error_message(status: StatusCode, body: &str, operation: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return operation.to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
            return format!("{}: {}", operation, truncate_error_text(message));
        }
    }
    if looks_like_html(trimmed) {
        let description = if status.is_server_error() {
            "GitHub service is temporarily unavailable"
        } else {
            "GitHub returned an unexpected HTML response"
        };
        return format!("{}: {}", operation, description);
    }
    format!("{}: {}", operation, truncate_error_text(trimmed))
}

fn looks_like_html(value: &str) -> bool {
    let normalized = value.trim_start().to_ascii_lowercase();
    normalized.starts_with("<!doctype html")
        || normalized.starts_with("<html")
        || normalized.contains("<body")
        || normalized.contains("<style")
}

fn truncate_error_text(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_API_ERROR_CHARS {
        return normalized;
    }
    let mut truncated = normalized
        .chars()
        .take(MAX_API_ERROR_CHARS.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_publication_responses(prepared:&super::super::archive::PreparedArchive,last:&str)->Vec<String> {
        let retired=format!("neriplayer-sync-v3-{}.zst","a".repeat(64));
        let mut responses=vec![response("200 OK",r#"{"tree":{"sha":"base-tree"}}"#),response("200 OK",&format!(r#"{{"truncated":false,"tree":[{{"path":"README.md","type":"blob","mode":"100644","sha":"unrelated"}},{{"path":"{retired}","type":"blob","mode":"100644","sha":"retired"}}]}}"#))];
        for index in 0..=prepared.objects.len(){responses.push(response("201 Created",&format!(r#"{{"sha":"blob-{index}"}}"#)));}
        responses.extend([response("201 Created",r#"{"sha":"new-tree"}"#),response("201 Created",r#"{"sha":"new-commit"}"#),response("200 OK",last)]);responses
    }

    #[tokio::test]
    async fn archive_publication_is_one_atomic_before_oid_update_and_preserves_unrelated_files() {
        let prepared=super::super::archive::prepare(&super::super::models::SyncData::default(),Some("publish".into())).unwrap();
        let (base,mut requests,server)=mock_server(archive_publication_responses(&prepared,r#"{"data":{"updateRefs":{"clientMutationId":"new-commit"}}}"#)).await;
        let api=GitHubApiClient::new_with_api_base(&loopback_client(),"fixture",&base);
        let head=GitHubHead{branch:"sync-data".into(),sha:"old-head".into(),repository_id:"repo-node".into()};
        assert_eq!(api.publish_archive("owner","repo",&head,&prepared,&std::collections::HashSet::new()).await.unwrap(),"new-commit");
        let count=prepared.objects.len()+6;let mut captured=Vec::new();for _ in 0..count{captured.push(requests.recv().await.unwrap());}
        let tree=&captured[captured.len()-3];assert!(tree.contains("\"base_tree\":\"base-tree\""));assert!(tree.contains("\"sha\":null"));assert!(!tree.contains("README.md"));
        let mutation=captured.last().unwrap();assert!(mutation.starts_with("POST /graphql "));assert!(mutation.contains("\"beforeOid\":\"old-head\""));assert!(mutation.contains("\"afterOid\":\"new-commit\""));assert!(mutation.contains("\"force\":false"));assert!(!captured.iter().any(|request|request.starts_with("PATCH ")||request.starts_with("PUT ")));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn atomic_stale_data_is_a_conflict_and_never_falls_back_to_rest_ref_patch() {
        let prepared=super::super::archive::prepare(&super::super::models::SyncData::default(),None).unwrap();
        let (base,mut requests,server)=mock_server(archive_publication_responses(&prepared,r#"{"errors":[{"type":"STALE_DATA","message":"branch changed"}]}"#)).await;
        let api=GitHubApiClient::new_with_api_base(&loopback_client(),"fixture",&base);
        let head=GitHubHead{branch:"main".into(),sha:"old-head".into(),repository_id:"repo-node".into()};
        assert!(api.publish_archive("owner","repo",&head,&prepared,&std::collections::HashSet::new()).await.unwrap_err().is_content_conflict());
        for _ in 0..prepared.objects.len()+6 {assert!(!requests.recv().await.unwrap().starts_with("PATCH "));}
        server.await.unwrap();
    }

    #[tokio::test]
    async fn fixed_commit_reads_keep_the_same_snapshot_ref_and_raw_bytes() {
        let (base,mut requests,server)=mock_server(vec![response("200 OK","manifest"),response("200 OK","object")]).await;
        let api=GitHubApiClient::new_with_api_base(&loopback_client(),"fixture",&base);
        for path in ["neriplayer-sync-v3.manifest","object.zst"] {api.get_file_at_ref("owner","repo",path,"fixed-head",100).await.unwrap();}
        for _ in 0..2 {let request=requests.recv().await.unwrap();assert!(request.lines().next().unwrap().contains("ref=fixed-head"));assert!(request.contains("application/vnd.github.raw+json"));}
        server.await.unwrap();
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    async fn mock_server(
        responses: Vec<String>,
    ) -> (String, mpsc::Receiver<String>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel(responses.len().max(1));
        let handle = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    let header_end = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|position| position + 4);
                    let Some(header_end) = header_end else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + content_length {
                        break;
                    }
                }
                let _ = request_tx
                    .send(String::from_utf8_lossy(&request).into_owned())
                    .await;
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        (format!("http://{}", address), request_rx, handle)
    }

    /// 测试必须绕开系统代理：mock server 监听 127.0.0.1，
    /// reqwest 默认会读取 macOS/Windows 的系统代理设置并把回环请求也发给代理
    fn loopback_client() -> Client {
        Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build loopback test client")
    }

    fn response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        )
    }

    #[tokio::test]
    async fn get_file_only_maps_not_found_to_none() {
        let (base, _, server) = mock_server(vec![response("404 Not Found", "{}")]).await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let result = api.get_file_content("owner", "repo", "backup.bin").await;

        assert!(matches!(result, Ok(None)));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_file_falls_back_to_raw_when_contents_api_omits_inline_content() {
        // Contents API 对 >1MB 的文件只回 size，content 为空串
        let (base, mut requests, server) = mock_server(vec![
            response(
                "200 OK",
                r#"{"size":2000000,"sha":"deadbeef","content":"","encoding":"none"}"#,
            ),
            response("200 OK", "RAW-BACKUP-PAYLOAD"),
        ])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let result = api
            .get_file_content("owner", "repo", "backup.bin")
            .await
            .unwrap();

        assert_eq!(
            result,
            Some((b"RAW-BACKUP-PAYLOAD".to_vec(), "deadbeef".to_string()))
        );
        let _json_request = requests.recv().await.unwrap();
        let raw_request = requests.recv().await.unwrap();
        assert!(raw_request.contains("application/vnd.github.raw"));
        // 追加第二个 Accept 会让 GitHub 仍按 JSON 返回，必须只带 raw 这一个
        assert!(!raw_request.contains("application/vnd.github+json"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_file_keeps_inline_content_when_present() {
        let (base, _, server) = mock_server(vec![response(
            "200 OK",
            r#"{"size":6,"sha":"abc123","content":"aW5saW5l","encoding":"base64"}"#,
        )])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let result = api
            .get_file_content("owner", "repo", "backup.bin")
            .await
            .unwrap();

        assert_eq!(result, Some((b"inline".to_vec(), "abc123".to_string())));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_file_propagates_server_errors() {
        let (base, _, server) = mock_server(vec![response(
            "500 Internal Server Error",
            r#"{"message":"temporary failure"}"#,
        )])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let error = api
            .get_file_content("owner", "repo", "backup.bin")
            .await
            .unwrap_err();

        assert!(matches!(error, GitHubApiError::Api { status: 500, .. }));
        server.await.unwrap();
    }

    /// 线格式：上传时 content 必须是「文件原始字节的 Base64」，只编一次
    ///
    /// 曾经这里传进来的已经是 Base64 文本，再编一次就成了双层 Base64——
    /// 流量白多 1/3，且 Android 解一次得到的是 Base64 文本而不是 GZIP 字节。
    /// 这个断言按字节比对，双端只要有一方改了层数就会立刻失败。
    #[tokio::test]
    async fn upload_base64_encodes_the_raw_bytes_exactly_once() {
        let (base, mut requests, server) = mock_server(vec![
            response("200 OK", r#"{"default_branch":"main"}"#),
            response("200 OK", r#"{"content":{"sha":"new-sha"}}"#),
        ])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        // 以 GZIP 魔数开头的原始字节：既非 UTF-8 文本，也不是 Base64
        let raw: &[u8] = &[0x1F, 0x8B, 0x08, 0x00, 0x00, 0xFF, 0xFE, 0x42];
        api.update_file_content("owner", "repo", "backup.bin", raw, "", "sync")
            .await
            .unwrap();

        let _repository_request = requests.recv().await.unwrap();
        let update_request = requests.recv().await.unwrap();
        let expected = BASE64.encode(raw);
        assert!(
            update_request.contains(&format!("\"content\":\"{expected}\"")),
            "content 必须正好是一层 Base64: {update_request}",
        );
        // 双层编码的特征：把结果再解一次仍然是合法 Base64 文本
        assert!(
            !update_request.contains(&BASE64.encode(expected.as_bytes())),
            "出现了双层 Base64",
        );
        server.await.unwrap();
    }

    /// 下载路径必须原样吐出文件字节，不做 UTF-8 转换
    ///
    /// 新格式的 backup.bin 是裸 GZIP，任何 String::from_utf8 都会把它判死。
    #[tokio::test]
    async fn download_returns_raw_bytes_for_non_utf8_payloads() {
        let raw: &[u8] = &[0x1F, 0x8B, 0x08, 0x00, 0x00, 0xFF, 0xFE, 0x42];
        let encoded = BASE64.encode(raw);
        let (base, _requests, server) = mock_server(vec![response(
            "200 OK",
            &format!(r#"{{"size":8,"content":"{encoded}","sha":"abc123"}}"#),
        )])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let result = api
            .get_file_content("owner", "repo", "backup.bin")
            .await
            .unwrap();

        assert_eq!(result, Some((raw.to_vec(), "abc123".to_string())));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn update_uses_default_branch_and_classifies_sha_conflict() {
        let (base, mut requests, server) = mock_server(vec![
            response("200 OK", r#"{"default_branch":"sync-data"}"#),
            response("409 Conflict", r#"{"message":"sha does not match"}"#),
        ])
        .await;
        let api = GitHubApiClient::new_with_api_base(&loopback_client(), "token", &base);

        let error = api
            .update_file_content("owner", "repo", "backup.bin", b"content", "old-sha", "sync")
            .await
            .unwrap_err();

        assert!(error.is_content_conflict());
        let _repository_request = requests.recv().await.unwrap();
        let update_request = requests.recv().await.unwrap();
        assert!(update_request.contains("\"branch\":\"sync-data\""));
        assert!(update_request.contains("\"sha\":\"old-sha\""));
        server.await.unwrap();
    }
}

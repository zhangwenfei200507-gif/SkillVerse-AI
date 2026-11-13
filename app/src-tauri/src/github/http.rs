use super::{
    credentials::Credentials,
    types::{ApiError, Outcome, digest, now},
};
use crate::db::Database;
use reqwest::{
    Client,
    header::{
        AUTHORIZATION, ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH,
        LAST_MODIFIED,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, watch};

#[derive(Clone)]
pub struct Cancellation(watch::Sender<bool>);
impl Cancellation {
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}
#[derive(Clone)]
pub struct RequestContext {
    pub cancel: Cancellation,
    pub requests: Arc<AtomicUsize>,
}
impl RequestContext {
    pub fn new() -> Self {
        Self {
            cancel: Cancellation::new(),
            requests: Arc::new(AtomicUsize::new(0)),
        }
    }
}
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CacheEntry {
    pub etag: Option<String>,
    pub payload_json: String,
    pub fetched_at_unix_ms: i64,
    pub last_modified: Option<String>,
    pub validated_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Default)]
#[serde(rename_all = "camelCase")]
pub struct RateState {
    pub scope: String,
    pub resource: String,
    pub remaining: Option<i64>,
    pub limit_value: Option<i64>,
    pub reset_at: i64,
    pub blocked_until: i64,
    pub last_request_at: i64,
    pub failures: i64,
}
#[derive(Clone)]
pub struct HttpConfig {
    pub base: String,
    pub timeout: Duration,
    pub core_gap: i64,
    pub search_gap: i64,
    pub retry_delay: Duration,
}
impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            base: "https://api.github.com".into(),
            timeout: Duration::from_secs(15),
            core_gap: 250,
            search_gap: 6100,
            retry_delay: Duration::from_millis(500),
        }
    }
}
pub struct GitHubHttp {
    pub db: Database,
    pub credentials: Arc<dyn Credentials>,
    client: Client,
    queue: Mutex<()>,
    config: HttpConfig,
}
impl GitHubHttp {
    pub fn new(db: Database, credentials: Arc<dyn Credentials>) -> Result<Self, ApiError> {
        Self::configured(db, credentials, HttpConfig::default())
    }
    fn configured(
        db: Database,
        credentials: Arc<dyn Credentials>,
        config: HttpConfig,
    ) -> Result<Self, ApiError> {
        let production = config.base == "https://api.github.com";
        if !production && !cfg!(test) {
            return Err(ApiError::new(
                "HOST",
                "The GitHub API host cannot be overridden.",
            ));
        }
        let client = Client::builder()
            .retry(reqwest::retry::never())
            .user_agent("SkillVerse-AI/0.1 (public read-only capability discovery)")
            .timeout(config.timeout)
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 4 {
                    attempt.error("redirect limit")
                } else if production
                    && (attempt.url().scheme() != "https"
                        || attempt.url().host_str() != Some("api.github.com"))
                {
                    attempt.error("GitHub redirect left the trusted API host")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| {
                ApiError::new("HTTP_CLIENT", "GitHub HTTP client could not initialize.")
            })?;
        Ok(Self {
            db,
            credentials,
            client,
            queue: Mutex::new(()),
            config,
        })
    }
    #[cfg(test)]
    pub fn for_test(db: Database, credentials: Arc<dyn Credentials>, config: HttpConfig) -> Self {
        Self::configured(db, credentials, config).unwrap()
    }
    pub async fn get(
        &self,
        path: &str,
        ttl: i64,
        force: bool,
        context: &RequestContext,
    ) -> Result<Outcome, ApiError> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains(['\r', '\n', '\0']) {
            return Err(ApiError::new("ENDPOINT", "Invalid GitHub endpoint."));
        }
        let token = self.credentials.read()?;
        let scope = if token.is_some() {
            "public-authenticated"
        } else {
            "public-anonymous"
        };
        let key = digest(format!("{scope}:GET:{path}").as_bytes());
        let started = now();
        let mut cached = self.db.gh_cache(&key).await?;
        if context.cancel.is_cancelled() {
            return Err(ApiError::new(
                "CANCELLED",
                "GitHub operation was cancelled.",
            ));
        }
        if !force
            && cached
                .as_ref()
                .is_some_and(|c| c.expires_at_unix_ms > started)
        {
            return cached_outcome(cached.as_ref().unwrap(), "cache", None);
        }
        let before_queue = cached.clone();
        let _serial = tokio::select! {guard=self.queue.lock()=>guard,_=context.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","GitHub operation was cancelled."))};
        // Re-read after taking the queue: concurrent identical calls share a result.
        cached = self.db.gh_cache(&key).await?;
        if cached.as_ref().is_some_and(|c| {
            c.expires_at_unix_ms > now() && (!force || before_queue.as_ref() != Some(c))
        }) {
            return cached_outcome(cached.as_ref().unwrap(), "cache", None);
        }
        let resource = if path.starts_with("/search/") {
            "search"
        } else {
            "core"
        };
        for attempt in 0..3u32 {
            let state = self.db.gh_rate(scope, resource).await?;
            let secondary = self.db.gh_rate(scope, "secondary").await?;
            let blocked = state.blocked_until.max(secondary.blocked_until).max(
                if state.remaining == Some(0) {
                    state.reset_at
                } else {
                    0
                },
            );
            if blocked > now() {
                let mut error = ApiError::new(
                    "RATE_LIMITED",
                    "GitHub requests are paused until the server's recovery time.",
                );
                error.retry_at = Some(blocked);
                return stale_or_error(cached.as_ref(), error);
            }
            let interval = if resource == "search" {
                if token.is_some() {
                    2100
                } else {
                    self.config.search_gap
                }
            } else {
                self.config.core_gap
            };
            let delay = (state.last_request_at + interval - now()).max(0) as u64;
            if delay > 0 {
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(delay))=>{},_=context.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","GitHub operation was cancelled."))}
            }
            if context
                .requests
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    if value < 36 { Some(value + 1) } else { None }
                })
                .is_err()
            {
                return stale_or_error(
                    cached.as_ref(),
                    ApiError::new(
                        "REQUEST_BUDGET",
                        "This inspection reached its bounded request budget. Saved candidates remain available.",
                    ),
                );
            }
            self.db.gh_request_started(scope, resource, now()).await?;
            let mut request = self
                .client
                .get(format!("{}{path}", self.config.base))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(secret) = token.as_ref() {
                let mut header = HeaderValue::from_str(&format!("Bearer {secret}"))
                    .map_err(|_| ApiError::new("INVALID_TOKEN", "The stored token is invalid."))?;
                header.set_sensitive(true);
                request = request.header(AUTHORIZATION, header);
            }
            if let Some(entry) = cached.as_ref() {
                if let Some(etag) = entry.etag.as_ref() {
                    request = request.header(IF_NONE_MATCH, etag);
                } else if let Some(date) = entry.last_modified.as_ref() {
                    request = request.header(IF_MODIFIED_SINCE, date);
                }
            }
            let response = tokio::select! {response=async{
                let mut response=request.send().await?;
                let status=response.status().as_u16();let headers=response.headers().clone();let mut data=Vec::new();
                let limit=if path.contains("/git/blobs/"){4_000_000}else{2_000_000};
                while let Some(chunk)=response.chunk().await? {if data.len()+chunk.len()>limit{return Ok::<_,reqwest::Error>((status,headers,None));}data.extend_from_slice(&chunk);}
                Ok((status,headers,Some(data)))
            }=>response,_=context.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","GitHub operation was cancelled."))};
            match response {
                Err(error) => {
                    if attempt < 2 {
                        tokio::select! {_=tokio::time::sleep(self.config.retry_delay*(1<<attempt))=>{},_=context.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","GitHub operation was cancelled."))};
                        continue;
                    }
                    return stale_or_error(
                        cached.as_ref(),
                        ApiError::new(
                            if error.is_timeout() {
                                "TIMEOUT"
                            } else {
                                "OFFLINE"
                            },
                            "GitHub could not be reached. Previously fetched data was retained.",
                        ),
                    );
                }
                Ok((status, headers, data)) => {
                    let response_text = data
                        .as_ref()
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .unwrap_or("");
                    let limited = status == 429
                        || (status == 403
                            && (header_i64(&headers, "x-ratelimit-remaining") == Some(0)
                                || response_text.to_ascii_lowercase().contains("rate limit")
                                || response_text.to_ascii_lowercase().contains("secondary")));
                    let mut rate = rate_headers(scope, resource, &headers, &state, now());
                    if limited {
                        let retry_at = rate_deadline(&headers, now(), state.failures);
                        rate.blocked_until = retry_at;
                        rate.failures = state.failures + 1;
                        self.db.gh_save_rate(&rate).await?;
                        if header_i64(&headers, "x-ratelimit-remaining") != Some(0) {
                            let mut global = rate.clone();
                            global.resource = "secondary".into();
                            self.db.gh_save_rate(&global).await?;
                        }
                        let mut error = ApiError::new(
                            "RATE_LIMITED",
                            "GitHub rate limiting paused this operation. Retry after the recorded recovery time.",
                        );
                        error.retry_at = Some(retry_at);
                        error.status = Some(status);
                        return stale_or_error(cached.as_ref(), error);
                    }
                    rate.failures = 0;
                    rate.blocked_until = 0;
                    self.db.gh_save_rate(&rate).await?;
                    if status == 304 {
                        if let Some(entry) = cached.as_mut() {
                            entry.validated_at_unix_ms = now();
                            entry.expires_at_unix_ms = now().saturating_add(ttl);
                            self.db.gh_save_cache(&key, entry).await?;
                            return cached_outcome(entry, "revalidated", None);
                        }
                        return Err(ApiError::new(
                            "INVALID_RESPONSE",
                            "GitHub returned 304 without a cached representation.",
                        ));
                    }
                    if status >= 500 && attempt < 2 {
                        tokio::select! {_=tokio::time::sleep(self.config.retry_delay*(1<<attempt))=>{},_=context.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","GitHub operation was cancelled."))};
                        continue;
                    }
                    if status != 200 {
                        // A later cache hit must not resurrect a resource that
                        // GitHub has explicitly reported missing.
                        if status == 404 {
                            if let Some(entry) = cached.as_mut() {
                                entry.expires_at_unix_ms = 0;
                                entry.etag = None;
                                entry.last_modified = None;
                                self.db.gh_save_cache(&key, entry).await?;
                            }
                        }
                        let mut error = ApiError::new(
                            match status {
                                404 => "NOT_FOUND",
                                401 => "UNAUTHORIZED",
                                403 => "FORBIDDEN",
                                422 => "INVALID_QUERY",
                                _ => "HTTP_ERROR",
                            },
                            "GitHub did not return a usable public resource. No candidate was removed.",
                        );
                        error.status = Some(status);
                        return if status >= 500 {
                            stale_or_error(cached.as_ref(), error)
                        } else {
                            Err(error)
                        };
                    }
                    let data = data.ok_or_else(|| {
                        ApiError::new(
                            "RESPONSE_LIMIT",
                            "GitHub's response exceeds the read-only inspection limit.",
                        )
                    })?;
                    if token
                        .as_ref()
                        .is_some_and(|secret| String::from_utf8_lossy(&data).contains(secret))
                    {
                        return Err(ApiError::new(
                            "UNSAFE_RESPONSE",
                            "A response containing credential material was rejected.",
                        ));
                    }
                    let body: Value = serde_json::from_slice(&data).map_err(|_| {
                        ApiError::new("INVALID_RESPONSE", "GitHub returned invalid JSON.")
                    })?;
                    if body["private"].as_bool() == Some(true) {
                        return Err(ApiError::new(
                            "PRIVATE",
                            "Private repository data is not saved by public discovery.",
                        ));
                    }
                    let fetched = now();
                    let entry = CacheEntry {
                        etag: header_string(&headers, ETAG.as_str()),
                        last_modified: header_string(&headers, LAST_MODIFIED.as_str()),
                        payload_json: serde_json::to_string(&body).unwrap(),
                        fetched_at_unix_ms: fetched,
                        validated_at_unix_ms: fetched,
                        expires_at_unix_ms: fetched.saturating_add(ttl),
                    };
                    self.db.gh_save_cache(&key, &entry).await?;
                    return Ok(Outcome {
                        body,
                        source: "network".into(),
                        fetched_at: fetched,
                        validated_at: fetched,
                        warning: None,
                    });
                }
            }
        }
        unreachable!()
    }
}
fn cached_outcome(
    entry: &CacheEntry,
    source: &str,
    warning: Option<ApiError>,
) -> Result<Outcome, ApiError> {
    Ok(Outcome {
        body: serde_json::from_str(&entry.payload_json)
            .map_err(|_| ApiError::new("CACHE", "Cached GitHub JSON is invalid."))?,
        source: source.into(),
        fetched_at: entry.fetched_at_unix_ms,
        validated_at: entry.validated_at_unix_ms,
        warning,
    })
}
fn stale_or_error(entry: Option<&CacheEntry>, error: ApiError) -> Result<Outcome, ApiError> {
    match entry {
        Some(entry) => cached_outcome(entry, "stale", Some(error)),
        None => Err(error),
    }
}
fn header_string(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() < 512)
        .map(str::to_string)
}
fn header_i64(headers: &HeaderMap, key: &str) -> Option<i64> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}
pub fn rate_deadline(headers: &HeaderMap, time: i64, failures: i64) -> i64 {
    let retry = header_string(headers, "retry-after").and_then(|v| {
        v.parse::<i64>()
            .ok()
            .map(|seconds| time.saturating_add(seconds.max(0).saturating_mul(1000)))
            .or_else(|| {
                httpdate::parse_http_date(&v)
                    .ok()
                    .and_then(|date| date.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
            })
    });
    let reset = if header_i64(headers, "x-ratelimit-remaining") == Some(0) {
        header_i64(headers, "x-ratelimit-reset").map(|s| s.saturating_mul(1000))
    } else {
        None
    };
    retry
        .into_iter()
        .chain(reset)
        .max()
        .unwrap_or_else(|| time + 60_000 * (1i64 << failures.clamp(0, 6)))
        .max(time + 1000)
}
fn rate_headers(
    scope: &str,
    resource: &str,
    headers: &HeaderMap,
    old: &RateState,
    time: i64,
) -> RateState {
    RateState {
        scope: scope.into(),
        resource: resource.into(),
        remaining: header_i64(headers, "x-ratelimit-remaining")
            .or(old.remaining.map(|n| (n - 1).max(0))),
        limit_value: header_i64(headers, "x-ratelimit-limit").or(old.limit_value),
        reset_at: header_i64(headers, "x-ratelimit-reset")
            .map(|s| s.saturating_mul(1000))
            .unwrap_or(old.reset_at),
        last_request_at: time,
        ..old.clone()
    }
}

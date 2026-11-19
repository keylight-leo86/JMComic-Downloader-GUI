//! 禁漫客户端：域名管理、请求重试、API 解密、图片下载。
//!
//! 对应 Python 版 `jm_client_impl.py` + `jm_client_interface.py`。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::config::{self, secret, ClientKind};
use crate::crypto;
use crate::error::{brief, JmError, Result};
use crate::events::{log, LogSink};
use crate::html_client;
use crate::http::build_http_client;
use crate::types::{pick_image_domain, Album, Episode, Photo};

/// 单个域名的重试退避（毫秒）。
fn backoff(attempt: usize) -> Duration {
    let millis = 120u64.saturating_mul(attempt as u64 + 1).min(1500);
    Duration::from_millis(millis)
}

pub struct JmClient {
    kind: ClientKind,
    http: reqwest::Client,
    sink: LogSink,
    retry: u32,
    /// 域名列表用 Arc 缓存，请求路径上只做一次引用计数，避免反复分配字符串。
    domains: RwLock<Arc<Vec<String>>>,
    domain_cursor: AtomicUsize,
    image_domains: RwLock<Arc<Vec<String>>>,
    image_cursor: AtomicUsize,
    /// 是否使用用户显式指定的图片域名（此时不再优先使用接口返回的 CDN 域名）
    image_pinned: bool,
    /// 预构建的请求头，避免每张图片重复分配
    api_base_headers: HeaderMap,
    image_headers: HeaderMap,
    app_version: RwLock<String>,
    scramble_cache: Mutex<HashMap<String, String>>,
}

impl JmClient {
    /// 创建客户端。
    ///
    /// `domains_override` / `image_domains_override` 非空时跳过自动更新，直接使用给定域名
    /// （用于自定义镜像或本地回环测试）。
    pub async fn new(
        kind: ClientKind,
        proxy: &str,
        retry: u32,
        sink: LogSink,
        domains_override: Option<Vec<String>>,
        image_domains_override: Option<Vec<String>>,
    ) -> Result<Arc<Self>> {
        let http = build_http_client(proxy)?;
        let mut domains: Vec<String> = match kind {
            ClientKind::Api => config::DEFAULT_API_DOMAINS,
            ClientKind::Html => config::DEFAULT_HTML_DOMAINS,
        }
        .iter()
        .map(|d| d.to_string())
        .collect();

        let mut image_domains: Vec<String> = config::DEFAULT_IMAGE_DOMAINS
            .iter()
            .map(|d| d.to_string())
            .collect();

        let pinned = domains_override
            .as_ref()
            .map(|list| !list.is_empty())
            .unwrap_or(false);

        let image_pinned = image_domains_override
            .as_ref()
            .map(|list| !list.is_empty())
            .unwrap_or(false);

        if let Some(list) = domains_override {
            let cleaned: Vec<String> = list
                .into_iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
            if !cleaned.is_empty() {
                domains = cleaned;
            }
        }

        if let Some(list) = image_domains_override {
            let cleaned: Vec<String> = list
                .into_iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
            if !cleaned.is_empty() {
                image_domains = cleaned;
            }
        }

        let api_base_headers = build_api_base_headers();
        let image_headers = build_image_headers();

        let client = Arc::new(Self {
            kind,
            http,
            sink,
            retry,
            domains: RwLock::new(Arc::new(domains)),
            domain_cursor: AtomicUsize::new(0),
            image_domains: RwLock::new(Arc::new(image_domains)),
            image_cursor: AtomicUsize::new(0),
            image_pinned,
            api_base_headers,
            image_headers,
            app_version: RwLock::new(config::APP_VERSION_API_FALLBACK.to_string()),
            scramble_cache: Mutex::new(HashMap::new()),
        });

        client.bootstrap(pinned).await;
        Ok(client)
    }

    pub fn kind(&self) -> ClientKind {
        self.kind
    }

    /// 当前接口域名列表（Arc 共享，克隆开销极小）。
    pub fn domains(&self) -> Arc<Vec<String>> {
        self.domains
            .read()
            .map(|d| d.clone())
            .unwrap_or_else(|_| Arc::new(Vec::new()))
    }

    fn set_domains(&self, list: Vec<String>) {
        if let Ok(mut guard) = self.domains.write() {
            *guard = Arc::new(list);
        }
    }

    /// 启动时初始化：刷新域名、获取 cookies（API 客户端）。
    async fn bootstrap(&self, pinned_domains: bool) {
        match self.kind {
            ClientKind::Api => {
                if !pinned_domains {
                    if let Some(list) = self.fetch_latest_api_domains().await {
                        if !list.is_empty() {
                            self.set_domains(list.clone());
                            log(
                                &self.sink,
                                format!("已自动更新 API 域名：{}", list.join(", ")),
                            );
                        }
                    }
                }
                // 移动端接口要求携带 cookies，/setting 同时返回最新的 APP 版本号
                match self.setting().await {
                    Ok(version) => {
                        let changed = self.update_app_version(&version);
                        log(
                            &self.sink,
                            if changed {
                                format!("已通过 /setting 更新 APP 版本号：{version}")
                            } else {
                                format!("API 初始化完成（APP 版本 {version}）")
                            },
                        );
                    }
                    Err(err) => {
                        log(
                            &self.sink,
                            format!("初始化 /setting 失败，继续使用默认配置：{}", brief(&err)),
                        );
                    }
                }
            }
            ClientKind::Html => {
                if !pinned_domains {
                    if let Some(list) = self.fetch_html_domains().await {
                        if !list.is_empty() {
                            self.set_domains(list.clone());
                            log(&self.sink, format!("网页端可用域名：{}", list.join(", ")));
                        }
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // 域名自动更新
    // ---------------------------------------------------------------------

    async fn fetch_latest_api_domains(&self) -> Option<Vec<String>> {
        for url in config::API_DOMAIN_SERVER_LIST {
            let fetched = tokio::time::timeout(Duration::from_secs(5), async {
                let resp = self.http.get(*url).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                resp.text().await.ok()
            })
            .await
            .ok()
            .flatten();

            let Some(text) = fetched else { continue };
            let mut trimmed = text.trim_start_matches(|c: char| !c.is_ascii());
            // 去掉开头所有非 ascii 字符（与 Python 版一致）
            while !trimmed.is_empty() && !trimmed.starts_with(|c: char| c.is_ascii()) {
                trimmed = &trimmed[trimmed.len()..];
            }
            if trimmed.is_empty() {
                continue;
            }

            let decoded =
                crypto::decode_resp_data_no_ts(trimmed, secret::API_DOMAIN_SERVER_SECRET).ok()?;
            let value: Value = serde_json::from_str(&decoded).ok()?;
            let servers: Vec<String> = value
                .get("Server")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if !servers.is_empty() {
                return Some(servers);
            }
        }
        None
    }

    async fn fetch_html_domains(&self) -> Option<Vec<String>> {
        let mut list: Vec<String> = Vec::new();

        let headers = header_map(config::html_headers(config::DEFAULT_HTML_DOMAINS[0]));
        let redirect = tokio::time::timeout(Duration::from_secs(8), async {
            let resp = self
                .http
                .get(config::JM_REDIRECT_URL)
                .headers(headers.clone())
                .send()
                .await
                .ok()?;
            Some(resp.url().to_string())
        })
        .await
        .ok()
        .flatten();

        if let Some(url) = redirect {
            if let Some(host) = html_client::host_from_url(&url) {
                list.push(host);
            }
        }

        let pub_html = tokio::time::timeout(Duration::from_secs(8), async {
            let resp = self
                .http
                .get(config::JM_PUB_URL)
                .headers(headers)
                .send()
                .await
                .ok()?;
            if !resp.status().is_success() {
                return None;
            }
            resp.text().await.ok()
        })
        .await
        .ok()
        .flatten();

        if let Some(html) = pub_html {
            for domain in html_client::extract_domain_list(&html) {
                if !list.contains(&domain) {
                    list.push(domain);
                }
            }
        }

        if list.is_empty() {
            None
        } else {
            Some(list)
        }
    }

    fn update_app_version(&self, version: &str) -> bool {
        if !compare_versions(version, &self.current_app_version()) {
            return false;
        }
        if let Ok(mut guard) = self.app_version.write() {
            guard.clear();
            guard.push_str(version);
        }
        true
    }

    fn current_app_version(&self) -> String {
        self.app_version
            .read()
            .map(|v| v.clone())
            .unwrap_or_else(|_| config::APP_VERSION_API_FALLBACK.to_string())
    }

    // ---------------------------------------------------------------------
    // 基础请求
    // ---------------------------------------------------------------------

    fn api_headers(&self, token: &str, tokenparam: &str) -> HeaderMap {
        let mut headers = self.api_base_headers.clone();
        if let Ok(val) = HeaderValue::from_str(token) {
            headers.insert(HeaderName::from_static("token"), val);
        }
        if let Ok(val) = HeaderValue::from_str(tokenparam) {
            headers.insert(HeaderName::from_static("tokenparam"), val);
        }
        headers
    }

    /// 带域名轮换 + 重试的文本请求。
    async fn request_text(
        &self,
        path: &str,
        secret_key: &str,
        expect_json: bool,
    ) -> Result<(String, i64)> {
        let domains = self.domains();
        if domains.is_empty() {
            return JmError::msg("没有可用的域名，请检查网络或代理设置");
        }

        let per_domain = self.retry as usize + 1;
        let start = self.domain_cursor.load(Ordering::Relaxed);
        let mut last_error = String::new();

        for round in 0..domains.len() {
            let index = (start + round) % domains.len();
            let domain = domains[index].clone();

            for attempt in 0..per_domain {
                let ts = crypto::now_ts();
                let version = self.current_app_version();
                let (token, tokenparam) = crypto::token_and_tokenparam(ts, &version, secret_key);
                let url = config::join_url(&domain, path);
                let headers = self.api_headers(&token, &tokenparam);

                if attempt > 0 || round > 0 {
                    log(
                        &self.sink,
                        format!(
                            "重试接口请求：域名=[{domain}] 次数=[{}/{}] 路径=[{}]",
                            attempt + 1,
                            per_domain,
                            path
                        ),
                    );
                }

                match self.get_text_once(&url, headers, expect_json).await {
                    Ok(text) => return Ok((text, ts)),
                    Err(err) => {
                        last_error = brief(&err);
                        if attempt + 1 < per_domain {
                            tokio::time::sleep(backoff(attempt as usize)).await;
                        }
                    }
                }
            }

            // 当前域名彻底失败，游标前移，让后续请求优先用下一个域名
            self.domain_cursor.fetch_add(1, Ordering::Relaxed);
        }

        Err(JmError::Msg(format!(
            "请求重试全部失败：{path}；最后错误：{last_error}"
        )))
    }

    async fn get_text_once(
        &self,
        url: &str,
        headers: HeaderMap,
        expect_json: bool,
    ) -> Result<String> {
        let resp = self.http.get(url).headers(headers).send().await?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let preview: String = body.chars().take(200).collect();
            let desc = config::error_status_desc(status.as_u16())
                .map(|d| format!("，原因: [{d}]"))
                .unwrap_or_default();
            return Err(JmError::Msg(format!(
                "HTTP {} {}: {}{}",
                status.as_u16(),
                url,
                preview,
                desc
            )));
        }

        let text = resp.text().await?;
        if text.trim().is_empty() {
            return Err(JmError::Msg(format!("响应为空: {url}")));
        }

        if expect_json {
            let first = text.chars().find(|c| !c.is_whitespace()).unwrap_or(' ');
            if first != '{' {
                // 命中封禁 / 内部错误提示
                for (content, reason) in config::ERROR_RESPONSE_TEXT {
                    if text.contains(content) {
                        return Err(JmError::Msg(format!("{reason}（{content}）")));
                    }
                }
                let preview: String = text.chars().take(200).collect();
                return Err(JmError::Msg(format!(
                    "响应不是 JSON 格式，强制重试: {preview}"
                )));
            }
        }

        Ok(text)
    }

    /// 网页端页面请求（无 token，带浏览器 headers）。
    async fn request_html(&self, path: &str) -> Result<String> {
        let domains = self.domains();
        if domains.is_empty() {
            return JmError::msg("没有可用的网页端域名");
        }

        let per_domain = self.retry as usize + 1;
        let mut last_error = String::new();

        for round in 0..domains.len() {
            let index = (self.domain_cursor.load(Ordering::Relaxed) + round) % domains.len();
            let domain = domains[index].clone();
            let url = config::join_url(&domain, path);
            let headers = header_map(config::html_headers(&domain));

            for attempt in 0..per_domain {
                match self.get_text_once(&url, headers.clone(), false).await {
                    Ok(text) => return Ok(text),
                    Err(err) => {
                        last_error = brief(&err);
                        if attempt + 1 < per_domain {
                            tokio::time::sleep(backoff(attempt as usize)).await;
                        }
                    }
                }
            }
            self.domain_cursor.fetch_add(1, Ordering::Relaxed);
        }

        Err(JmError::Msg(format!(
            "网页请求重试全部失败：{path}；最后错误：{last_error}"
        )))
    }

    // ---------------------------------------------------------------------
    // 详情接口
    // ---------------------------------------------------------------------

    pub async fn album(&self, id: &str) -> Result<Album> {
        match self.kind {
            ClientKind::Api => {
                let (text, ts) = self
                    .request_text(&format!("/album?id={id}"), secret::APP_TOKEN_SECRET, true)
                    .await?;
                let value = decode_api_body(&text, ts)?;
                if !has_name(&value) {
                    return JmError::missing(format!("本子 {id}"));
                }
                album_from_json(&value)
            }
            ClientKind::Html => {
                let html = self.request_html(&format!("/album/{id}")).await?;
                html_client::parse_album_html(&html)
            }
        }
    }

    pub async fn photo_detail(&self, id: &str, album_hint: Option<Album>) -> Result<Photo> {
        match self.kind {
            ClientKind::Api => {
                let (text, ts) = self
                    .request_text(&format!("/chapter?id={id}"), secret::APP_TOKEN_SECRET, true)
                    .await?;
                let value = decode_api_body(&text, ts)?;
                if !has_name(&value) {
                    return JmError::missing(format!("章节 {id}"));
                }

                let mut photo = photo_from_json(&value)?;
                let album_id = photo.album_id.clone();
                let need_album = album_hint.is_none();
                if let Some(album) = album_hint {
                    photo.album = Some(album);
                }

                // 两个附加请求并发执行（Python 版默认是串行）
                let (scramble, album_result) =
                    tokio::join!(self.scramble_id(&photo.id, &album_id), async {
                        if need_album {
                            Some(self.album(&album_id).await)
                        } else {
                            None
                        }
                    });

                photo.scramble_id = scramble?;
                if let Some(result) = album_result {
                    photo.album = Some(result?);
                }
                Ok(photo)
            }
            ClientKind::Html => {
                let html = self.request_html(&format!("/photo/{id}")).await?;
                let mut photo = html_client::parse_photo_html(&html)?;
                let need_album = album_hint.is_none();
                if let Some(album) = album_hint {
                    photo.album = Some(album);
                }
                if need_album {
                    photo.album = Some(self.album(&photo.album_id).await?);
                }
                Ok(photo)
            }
        }
    }

    /// 获取章节的 scramble_id（带缓存）。
    pub async fn scramble_id(&self, photo_id: &str, album_id: &str) -> Result<String> {
        if let Some(hit) = self.scramble_cache_get(photo_id) {
            return Ok(hit);
        }
        if let Some(hit) = self.scramble_cache_get(album_id) {
            self.scramble_cache_put(photo_id, &hit);
            return Ok(hit);
        }

        let ts = crypto::now_ts();
        let path = format!(
            "/chapter_view_template?id={photo_id}&mode=vertical&page=0&app_img_shunt=1&express=off&v={ts}"
        );
        let result = self
            .request_text(&path, secret::APP_TOKEN_SECRET_2, false)
            .await;

        let scramble = match result {
            Ok((text, _)) => extract_scramble_id(&text),
            Err(err) => {
                log(
                    &self.sink,
                    format!("获取 scramble_id 失败，使用默认值：{}", brief(&err)),
                );
                None
            }
        };

        let scramble = scramble.unwrap_or_else(|| config::SCRAMBLE_220980.to_string());
        self.scramble_cache_put(photo_id, &scramble);
        if !album_id.is_empty() {
            self.scramble_cache_put(album_id, &scramble);
        }
        Ok(scramble)
    }

    fn scramble_cache_get(&self, key: &str) -> Option<String> {
        self.scramble_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).cloned())
    }

    fn scramble_cache_put(&self, key: &str, value: &str) {
        if let Ok(mut cache) = self.scramble_cache.lock() {
            cache.insert(key.to_string(), value.to_string());
        }
    }

    async fn setting(&self) -> Result<String> {
        let (text, ts) = self
            .request_text("/setting", secret::APP_TOKEN_SECRET, true)
            .await?;
        let value = decode_api_body(&text, ts)?;
        let version = value
            .get("jm3_version")
            .and_then(|v| v.as_str())
            .unwrap_or(config::APP_VERSION_API_FALLBACK)
            .to_string();
        Ok(version)
    }

    // ---------------------------------------------------------------------
    // 图片
    // ---------------------------------------------------------------------

    /// 下载单张图片；失败时轮换 CDN 域名，并尝试带 `?v=` 参数的地址。
    pub async fn fetch_image(&self, photo: &Photo, index: usize) -> Result<Vec<u8>> {
        let available: Arc<Vec<String>> = self
            .image_domains
            .read()
            .map(|list| list.clone())
            .unwrap_or_else(|_| Arc::new(Vec::new()));

        // 候选域名：用户显式指定时只用指定列表；否则接口返回的域名优先。
        // 起点按图片序号轮换，把大量图片均匀分摊到多个 CDN。
        let mut candidates: Vec<&str> = Vec::with_capacity(available.len() + 1);
        if self.image_pinned {
            candidates.extend(available.iter().map(String::as_str));
        } else {
            if !photo.image_domain.is_empty() {
                candidates.push(photo.image_domain.as_str());
            }
            candidates.extend(
                available
                    .iter()
                    .map(String::as_str)
                    .filter(|domain| *domain != photo.image_domain),
            );
        }
        if candidates.is_empty() {
            candidates.push(pick_image_domain(index));
        }

        let start = self.image_cursor.load(Ordering::Relaxed) % candidates.len();
        let ordered: Vec<&str> = candidates[start..]
            .iter()
            .chain(candidates[..start].iter())
            .copied()
            .collect();

        let mut last_error = String::new();
        let mut attempts = 0usize;
        let max_attempts = (self.retry as usize + 2).clamp(2, 10);

        'outer: for (position, domain) in ordered.iter().enumerate() {
            for attempt in 0..=self.retry.min(3) {
                if attempts >= max_attempts {
                    break 'outer;
                }
                attempts += 1;

                let base = photo.image_url_with_domain(index, domain);
                let url = if attempt == 0 {
                    base
                } else {
                    format!("{base}?v={}", crypto::now_ts())
                };

                match self.get_image_once(&url).await {
                    Ok(bytes) if !bytes.is_empty() => {
                        if !is_probably_image(&bytes) {
                            last_error = format!("响应不是图片数据: {url}");
                        } else {
                            return Ok(bytes);
                        }
                    }
                    Ok(_) => last_error = format!("图片响应为空: {url}"),
                    Err(err) => last_error = brief(&err),
                }

                tokio::time::sleep(backoff(attempt as usize)).await;
            }

            if position + 1 < ordered.len() {
                log(
                    &self.sink,
                    format!(
                        "图片 CDN 切换：{domain} → {} ({}/{})",
                        ordered[position + 1],
                        position + 2,
                        ordered.len()
                    ),
                );
            }
        }

        // 记录一次游标推进，让后续图片优先使用其它域名
        self.image_cursor.fetch_add(1, Ordering::Relaxed);

        Err(JmError::Msg(format!(
            "图片下载失败：章节 {} 第 {} 张；最后错误：{last_error}",
            photo.id,
            index + 1
        )))
    }

    async fn get_image_once(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(url)
            .headers(self.image_headers.clone())
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(JmError::Msg(format!("HTTP {}", status.as_u16())));
        }
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// 测试用：直接访问一个 URL 并返回文本。
    pub async fn plain_text(&self, url: &str) -> Result<String> {
        let resp = self
            .http
            .get(url)
            .headers(header_map(config::html_headers(
                config::DEFAULT_HTML_DOMAINS[0],
            )))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(JmError::Msg(format!("HTTP {}", resp.status().as_u16())));
        }
        Ok(resp.text().await?)
    }
}

fn header_map(pairs: Vec<(String, String)>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (key, value) in pairs {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            headers.insert(name, val);
        }
    }
    headers
}

/// 预构建移动端接口的公共请求头（token 每次请求再补）。
fn build_api_base_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (key, value) in config::app_headers() {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, val);
        }
    }
    headers
}

/// 预构建图片请求头。
fn build_image_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (key, value) in config::app_image_headers() {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            headers.insert(name, val);
        }
    }
    for (key, value) in config::app_headers() {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, val);
        }
    }
    headers
}

fn is_probably_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    let jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    let png = bytes.starts_with(&[0x89, b'P', b'N', b'G']);
    let gif = bytes.starts_with(b"GIF8");
    let webp = bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    jpeg || png || gif || webp
}

fn has_name(value: &Value) -> bool {
    value.get("name").map(|n| !n.is_null()).unwrap_or(false)
}

fn extract_scramble_id(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"var scramble_id = (\d+);").ok()?;
    re.captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 宽松解析 JSON：允许前后有无关字符。
fn parse_json_loose(text: &str) -> Result<Value> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed)
            .map_err(|e| JmError::Msg(format!("JSON 解析失败: {e}")));
    }
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    if let (Some(start), Some(end)) = (start, end) {
        if start < end {
            return serde_json::from_str(&trimmed[start..=end])
                .map_err(|e| JmError::Msg(format!("JSON 解析失败: {e}")));
        }
    }
    JmError::msg(format!(
        "未解析出 JSON 数据: {}",
        trimmed.chars().take(200).collect::<String>()
    ))
}

/// 解析接口响应：`{code, data(base64+ AES)}`。
pub fn decode_api_body(text: &str, ts: i64) -> Result<Value> {
    let root = parse_json_loose(text)?;
    let code = root.get("code").and_then(|c| c.as_i64()).unwrap_or(200);
    if code != 200 {
        let message = root
            .get("errorMsg")
            .or_else(|| root.get("msg"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return JmError::msg(format!("接口返回错误 code={code}: {message}"));
    }

    let data = root.get("data").cloned().unwrap_or(Value::Null);
    let raw = match data {
        Value::String(raw) => raw,
        _ => return JmError::missing("接口未返回 data（可能本子不存在或已下架）"),
    };

    let plain = crypto::decode_resp_data(&raw, ts, secret::APP_DATA_SECRET)?;
    parse_json_loose(&plain)
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn get_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(as_text)
}

fn get_u32(value: &Value, key: &str, default: u32) -> u32 {
    get_text(value, key)
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(as_text)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 解析移动端 album JSON。
pub fn album_from_json(value: &Value) -> Result<Album> {
    let id = get_text(value, "id")
        .or_else(|| get_text(value, "album_id"))
        .ok_or_else(|| JmError::Msg("album JSON 缺少 id".to_string()))?;
    let name = get_text(value, "name").unwrap_or_default();

    let mut episodes: Vec<Episode> = value
        .get("series")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|chapter| {
                    let photo_id = get_text(chapter, "id")?;
                    let index = get_u32(chapter, "sort", 1);
                    let name = get_text(chapter, "name").unwrap_or_default();
                    Some(Episode {
                        photo_id,
                        index,
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    episodes.sort_by_key(|episode| episode.index);
    episodes.dedup_by_key(|episode| episode.index);

    if episodes.is_empty() {
        episodes.push(Episode {
            photo_id: id.clone(),
            index: 1,
            name: name.clone(),
        });
    }

    Ok(Album {
        id,
        name: name.trim().to_string(),
        authors: string_array(value, "author"),
        tags: string_array(value, "tags"),
        description: get_text(value, "description")
            .unwrap_or_default()
            .trim()
            .to_string(),
        page_count: get_u32(value, "page_count", 0),
        scramble_id: get_text(value, "scramble_id").unwrap_or_else(|| "0".to_string()),
        episodes,
    })
}

/// 解析移动端 chapter（photo）JSON。
pub fn photo_from_json(value: &Value) -> Result<Photo> {
    let id = get_text(value, "id")
        .or_else(|| get_text(value, "photo_id"))
        .ok_or_else(|| JmError::Msg("chapter JSON 缺少 id".to_string()))?;
    let name = get_text(value, "name")
        .unwrap_or_default()
        .trim()
        .to_string();

    let images: Vec<String> = value
        .get("images")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(as_text)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let series_id = get_text(value, "series_id")
        .and_then(|text| text.trim().parse::<i64>().ok())
        .unwrap_or(0);

    // sort 来自 series 中与自身 id 匹配的条目
    let sort = value
        .get("series")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|chapter| {
                let chapter_id = get_text(chapter, "id")?;
                if chapter_id == id {
                    Some(get_u32(chapter, "sort", 1))
                } else {
                    None
                }
            })
        })
        .unwrap_or(1)
        .max(1);

    let index = if series_id == 0 && sort == 2 { 1 } else { sort };

    let tags = match value.get("tags") {
        Some(Value::String(text)) => text
            .split(|c: char| c.is_whitespace() || c == ',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => string_array(value, "tags"),
    };

    let album_id = if series_id == 0 {
        id.clone()
    } else {
        series_id.to_string()
    };

    let seed: usize = id
        .bytes()
        .fold(0usize, |acc, b| acc.wrapping_add(b as usize));

    Ok(Photo {
        id,
        name,
        album_id,
        index,
        sort,
        series_id,
        tags,
        scramble_id: get_text(value, "scramble_id").unwrap_or_else(|| "0".to_string()),
        image_domain: pick_image_domain(seed).to_string(),
        images,
        album: None,
    })
}

/// 版本号比较：`true` 表示 v1 比 v2 新。
fn compare_versions(v1: &str, v2: &str) -> bool {
    fn parts(text: &str) -> Vec<u64> {
        text.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    }
    let mut left = parts(v1);
    let mut right = parts(v2);
    let len = left.len().max(right.len());
    left.resize(len, 0);
    right.resize(len, 0);
    left > right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(compare_versions("2.0.29", "2.0.28"));
        assert!(compare_versions("2.1", "2.0.28"));
        assert!(!compare_versions("2.0.28", "2.0.28"));
        assert!(!compare_versions("1.9.9", "2.0.0"));
    }

    #[test]
    fn parses_album_json() {
        let value: Value = serde_json::from_str(
            r#"{
                "id": 123,
                "name": "测试本子",
                "author": ["作者A"],
                "tags": ["全彩", "中文"],
                "description": null,
                "series": [
                    {"id": "200", "name": "第二话", "sort": "2"},
                    {"id": "100", "name": "第一话", "sort": "1"},
                    {"id": "100b", "name": "重复第一话", "sort": "1"}
                ]
            }"#,
        )
        .unwrap();

        let album = album_from_json(&value).unwrap();
        assert_eq!(album.id, "123");
        assert_eq!(album.name, "测试本子");
        assert_eq!(album.author(), "作者A");
        assert_eq!(album.tags, vec!["全彩", "中文"]);
        assert_eq!(album.episodes.len(), 2);
        assert_eq!(album.episodes[0].photo_id, "100");
        assert_eq!(album.episodes[1].photo_id, "200");
    }

    #[test]
    fn album_without_series_becomes_single_chapter() {
        let value: Value =
            serde_json::from_str(r#"{"id": 42, "name": "单章", "series": []}"#).unwrap();
        let album = album_from_json(&value).unwrap();
        assert_eq!(album.episodes.len(), 1);
        assert_eq!(album.episodes[0].photo_id, "42");
        assert!(album.is_single_chapter());
    }

    #[test]
    fn parses_photo_json() {
        let value: Value = serde_json::from_str(
            r#"{
                "id": 413446,
                "name": "第2话",
                "images": ["00001.webp", "00002.webp"],
                "series_id": "400222",
                "tags": "全彩 中文",
                "series": [{"id": "413446", "name": "第2话", "sort": "2"}]
            }"#,
        )
        .unwrap();

        let photo = photo_from_json(&value).unwrap();
        assert_eq!(photo.id, "413446");
        assert_eq!(photo.album_id, "400222");
        assert_eq!(photo.index, 2);
        assert_eq!(photo.images.len(), 2);
        assert_eq!(photo.tags, vec!["全彩", "中文"]);
        assert!(!photo.is_gif(0));
        assert_eq!(photo.image_stem(0), "00001");
        assert_eq!(photo.image_suffix(0), ".webp");
    }

    #[test]
    fn single_chapter_index_is_one() {
        let value: Value = serde_json::from_str(
            r#"{"id": 7, "name": "x", "images": ["1.png"], "series_id": 0, "series": []}"#,
        )
        .unwrap();
        let photo = photo_from_json(&value).unwrap();
        assert_eq!(photo.album_id, "7");
        assert_eq!(photo.sort, 1);
        assert_eq!(photo.index, 1);
        assert!(photo.is_single_album());
    }

    #[test]
    fn loose_json_parsing() {
        let value = parse_json_loose("prefix {\"a\": 1} suffix").unwrap();
        assert_eq!(value["a"], 1);
        assert!(parse_json_loose("no json here").is_err());
    }

    #[test]
    fn detects_json_error_response() {
        let err = decode_api_body(r#"{"code":400,"errorMsg":"参数错误"}"#, 0).unwrap_err();
        assert!(err.to_string().contains("参数错误"));
        assert!(decode_api_body(r#"{"code":200,"data":[]}"#, 0).is_err());
    }

    #[test]
    fn image_signature_check() {
        assert!(is_probably_image(&[
            0xFF, 0xD8, 0xFF, 1, 2, 3, 4, 5, 6, 7, 8, 9
        ]));
        assert!(is_probably_image(b"RIFF0000WEBPxxxx"));
        assert!(!is_probably_image(b"<html><body>"));
        assert!(!is_probably_image(b""));
    }
}

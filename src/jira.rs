use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub created: DateTime<Utc>,
    pub content: String,
    #[serde(rename = "mimeType", default)]
    pub mime_type: String,
}

#[derive(Debug, Clone)]
pub struct IssueInfo {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone)]
pub struct IssueSummary {
    pub key: String,
    pub summary: String,
    pub status: String,
}

#[derive(Deserialize)]
struct JiraIssueResponse {
    key: String,
    fields: JiraFields,
}

#[derive(Deserialize)]
struct JiraFields {
    #[serde(default)]
    summary: String,
    status: JiraStatus,
    #[serde(default)]
    attachment: Vec<JiraAttachment>,
}

#[derive(Deserialize)]
struct JiraStatus {
    name: String,
}

#[derive(Deserialize)]
struct JiraAttachment {
    id: String,
    filename: String,
    #[serde(default)]
    size: u64,
    #[serde(deserialize_with = "deserialize_jira_date", default = "epoch")]
    created: DateTime<Utc>,
    #[serde(default)]
    content: String,
    #[serde(rename = "mimeType", default)]
    mime_type: String,
}

/// JIRA sends dates as "2024-01-15T10:30:00.000+0000" (no colon in offset).
fn deserialize_jira_date<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.3f%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    Err(serde::de::Error::custom(format!("Cannot parse date: {s}")))
}

fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

pub struct JiraClient {
    client: Client,
    config: AppConfig,
}

impl JiraClient {
    pub fn new(config: AppConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    fn base_url(&self) -> String {
        // Strip any extra path — only keep scheme + host (+ optional port)
        let url = self.config.jira_url.trim_end_matches('/');
        if let Ok(parsed) = url::Url::parse(url) {
            let mut base = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            if let Some(port) = parsed.port() {
                base.push_str(&format!(":{port}"));
            }
            // Keep context path if present (e.g. /jira for Jira Server)
            let path = parsed.path().trim_end_matches('/');
            if !path.is_empty() && path != "/" {
                base.push_str(path);
            }
            base
        } else {
            url.to_string()
        }
    }

    fn auth(&self) -> reqwest::header::HeaderValue {
        use base64::Engine;
        let creds = format!("{}:{}", self.config.email, self.config.api_token);
        let encoded = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
        reqwest::header::HeaderValue::from_str(&format!("Basic {encoded}")).unwrap()
    }

    /// Returns (status, content_type, body)
    async fn get_raw(&self, url: &str) -> Result<(reqwest::StatusCode, String, String), String> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::AUTHORIZATION, self.auth())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}\nURL: {url}"))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await.unwrap_or_default();
        Ok((status, content_type, body))
    }

    fn check_html_response(status: reqwest::StatusCode, content_type: &str, body: &str, url: &str) -> Option<String> {
        let is_html = content_type.contains("text/html")
            || body.trim_start().starts_with("<!doctype")
            || body.trim_start().starts_with("<html");
        if is_html {
            Some(format!(
                "Got an HTML page instead of JSON (status: {status}).\n\
                 This usually means:\n\
                 • SSO/login redirect — credentials not accepted by the server\n\
                 • Wrong JIRA URL — use only the base URL, e.g. https://company.atlassian.net\n\
                 • Jira Server with context path — try https://company.atlassian.net/jira\n\
                 URL called: {url}"
            ))
        } else {
            None
        }
    }

    pub async fn test_connection(&self) -> Result<String, String> {
        // Try API v3 first (Cloud), fall back to v2 (Server/Data Center)
        for api_ver in &["3", "2"] {
            let url = format!("{}/rest/api/{}/myself", self.base_url(), api_ver);
            let (status, ct, body) = self.get_raw(&url).await?;

            if let Some(err) = Self::check_html_response(status, &ct, &body, &url) {
                return Err(err);
            }

            if status.is_success() {
                let parsed: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| format!("Parse error: {e}\nBody: {}", &body[..body.len().min(300)]))?;
                let name = parsed["displayName"].as_str().unwrap_or("unknown");
                return Ok(format!("Connected as: {name} (API v{api_ver})"));
            }
        }
        Err(format!(
            "Authentication failed.\nCheck your email and API token.\nJIRA URL: {}",
            self.base_url()
        ))
    }

    pub async fn fetch_my_issues(&self) -> Result<Vec<IssueSummary>, String> {
        // JQL: all unresolved issues assigned to the current user, newest first
        let jql = "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";
        let encoded_jql = url::form_urlencoded::byte_serialize(jql.as_bytes()).collect::<String>();

        // Try the new /search/jql endpoint first (required as of 2025),
        // fall back to the old /search for on-prem JIRA Server/Data Center.
        for endpoint in &["rest/api/3/search/jql", "rest/api/2/search"] {
            let url = format!(
                "{}/{}?jql={}&fields=summary,status&maxResults=100",
                self.base_url(),
                endpoint,
                encoded_jql
            );

            let (status, ct, body) = self.get_raw(&url).await?;

            if let Some(err) = Self::check_html_response(status, &ct, &body, &url) {
                return Err(err);
            }

            if (status == reqwest::StatusCode::NOT_FOUND
                || status == reqwest::StatusCode::GONE)
                && *endpoint == "rest/api/3/search/jql"
            {
                continue;
            }

            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", &body[..body.len().min(300)]));
            }

            #[derive(Deserialize)]
            struct SearchResponse {
                issues: Vec<SearchIssue>,
            }
            #[derive(Deserialize)]
            struct SearchIssue {
                key: String,
                fields: SearchFields,
            }
            #[derive(Deserialize)]
            struct SearchFields {
                #[serde(default)]
                summary: String,
                status: JiraStatus,
            }

            let resp: SearchResponse = serde_json::from_str(&body).map_err(|e| {
                format!("Failed to parse search response: {e}\nRaw: {}", &body[..body.len().min(300)])
            })?;

            return Ok(resp
                .issues
                .into_iter()
                .map(|i| IssueSummary {
                    key: i.key,
                    summary: i.fields.summary,
                    status: i.fields.status.name,
                })
                .collect());
        }

        Ok(vec![])
    }

    pub async fn fetch_issue(&self, key: &str) -> Result<IssueInfo, String> {
        // Try API v3 first, fall back to v2
        for api_ver in &["3", "2"] {
            let url = format!(
                "{}/rest/api/{}/issue/{}?fields=summary,status,attachment",
                self.base_url(),
                api_ver,
                key
            );

            let (status, ct, body) = self.get_raw(&url).await?;

            if let Some(err) = Self::check_html_response(status, &ct, &body, &url) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && *api_ver == "3" {
                continue; // try v2
            }

            if !status.is_success() {
                return Err(format!("HTTP {status}\nURL: {url}\nBody: {}", &body[..body.len().min(300)]));
            }

            let issue: JiraIssueResponse = serde_json::from_str(&body).map_err(|e| {
                let snippet = &body[..body.len().min(500)];
                format!("Failed to parse response (API v{api_ver}): {e}\nRaw: {snippet}")
            })?;

            let attachments = issue
                .fields
                .attachment
                .into_iter()
                .map(|a| Attachment {
                    id: a.id,
                    filename: a.filename,
                    size: a.size,
                    created: a.created,
                    content: a.content,
                    mime_type: a.mime_type,
                })
                .collect();

            return Ok(IssueInfo {
                key: issue.key,
                summary: issue.fields.summary,
                status: issue.fields.status.name,
                attachments,
            });
        }

        Err(format!("Issue {} not found on {}", key, self.base_url()))
    }

    pub async fn fetch_issue_status(&self, key: &str) -> Result<String, String> {
        for api_ver in &["3", "2"] {
            let url = format!(
                "{}/rest/api/{}/issue/{}?fields=status",
                self.base_url(),
                api_ver,
                key
            );

            let (status, ct, body) = self.get_raw(&url).await?;

            if let Some(err) = Self::check_html_response(status, &ct, &body, &url) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && *api_ver == "3" {
                continue;
            }

            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", &body[..body.len().min(200)]));
            }

            let issue: JiraIssueResponse = serde_json::from_str(&body).map_err(|e| {
                format!("Parse error: {e}")
            })?;

            return Ok(issue.fields.status.name);
        }

        Err(format!("Issue {} not found", key))
    }

    pub async fn download_attachment(
        &self,
        url: &str,
        on_progress: impl Fn(u64, u64) + Send + 'static,
    ) -> Result<bytes::Bytes, String> {
        use futures::StreamExt;

        let resp = self
            .client
            .get(url)
            .header(reqwest::header::AUTHORIZATION, self.auth())
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut buf = bytes::BytesMut::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
            downloaded += chunk.len() as u64;
            buf.extend_from_slice(&chunk);
            on_progress(downloaded, total);
        }

        Ok(buf.freeze())
    }
}

/// Parse issue key from either "PROJ-123" or full JIRA URL
pub fn parse_issue_key(input: &str) -> Option<String> {
    let input = input.trim();
    if input.starts_with("http") {
        let parts: Vec<&str> = input.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            if (*part == "browse" || *part == "issues") && i + 1 < parts.len() {
                let key = parts[i + 1].split('?').next().unwrap_or("");
                if is_valid_issue_key(key) {
                    return Some(key.to_uppercase());
                }
            }
        }
        if let Some(last) = parts.last() {
            let key = last.split('?').next().unwrap_or("");
            if is_valid_issue_key(key) {
                return Some(key.to_uppercase());
            }
        }
        None
    } else if is_valid_issue_key(input) {
        Some(input.to_uppercase())
    } else {
        None
    }
}

fn is_valid_issue_key(s: &str) -> bool {
    if let Some(dash_pos) = s.find('-') {
        let prefix = &s[..dash_pos];
        let suffix = &s[dash_pos + 1..];
        !prefix.is_empty()
            && !suffix.is_empty()
            && prefix.chars().all(|c| c.is_ascii_alphabetic())
            && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

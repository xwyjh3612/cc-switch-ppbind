//! Proxy Session - 请求会话管理
//!
//! 为每个代理请求创建会话上下文，在整个请求生命周期中跟踪状态和元数据。
//!
//! ## Session ID 提取
//!
//! 支持从客户端请求中提取 Session ID，用于关联同一对话的多个请求：
//! - Claude: 从 `metadata.user_id` (格式: `user_xxx_session_yyy`) 或 `metadata.session_id` 提取
//! - Codex: 从 headers 中的 `session_id` / `x-session-id` 或 `metadata.session_id` 提取
//! - Grok Build: 从 headers 中的 `x-grok-conv-id` / `x-grok-session-id` 提取
//! - 其他: 生成新的 UUID

use axum::http::HeaderMap;
use uuid::Uuid;

// ============================================================================
// Session ID 提取器
// ============================================================================

/// Session ID 来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdSource {
    /// 从 metadata.user_id 提取 (Claude)
    MetadataUserId,
    /// 从 metadata.session_id 提取
    MetadataSessionId,
    /// 从 headers 提取
    Header,
    /// 新生成
    Generated,
}

/// Session ID 提取结果
#[derive(Debug, Clone)]
pub struct SessionIdResult {
    /// 提取或生成的 Session ID
    pub session_id: String,
    /// Session ID 来源
    pub source: SessionIdSource,
    /// 是否为客户端提供的 ID（非新生成）
    pub client_provided: bool,
}

/// 从请求中提取或生成 Session ID
///
/// 轻量化实现，仅提取 session_id 用于日志记录，不做复杂的 Session 管理。
///
/// ## 提取优先级
///
/// ### Claude 请求
/// 1. `metadata.user_id` (格式: `user_xxx_session_yyy`) → 提取 `yyy` 部分
/// 2. `metadata.session_id` → 直接使用
/// 3. 生成新 UUID
///
/// ### Codex 请求
/// 1. Headers: `session_id` 或 `x-session-id`
/// 2. `metadata.session_id`
/// 3. 生成新 UUID
///
/// ### Grok Build 请求
/// 1. Headers: `x-grok-conv-id` 或 `x-grok-session-id`
/// 2. `metadata.session_id`
/// 3. 生成新 UUID
///
/// ## 示例
///
/// ```ignore
/// let result = extract_session_id(&headers, &body, "claude");
/// println!("Session ID: {} (from {:?})", result.session_id, result.source);
/// ```
pub fn extract_session_id(
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_format: &str,
) -> SessionIdResult {
    if client_format == "claude" {
        if let Some(result) = extract_claude_session(headers, body) {
            return result;
        }
    }

    // Responses 请求特殊处理。Grok Build 使用与 Codex 相同的客户端协议，
    // 但保留独立前缀，避免统计和缓存键跨应用碰撞。
    if matches!(client_format, "codex" | "openai" | "grokbuild") {
        let prefix = if client_format == "grokbuild" {
            "grokbuild"
        } else {
            "codex"
        };
        if let Some(result) = extract_responses_session(headers, body, prefix) {
            return result;
        }
    }

    // Claude 请求：从 metadata 提取
    if let Some(result) = extract_from_metadata(body) {
        return result;
    }

    // 兜底：生成新 Session ID
    generate_new_session_id()
}

/// 提取 Claude Session ID
fn extract_claude_session(
    headers: &HeaderMap,
    body: &serde_json::Value,
) -> Option<SessionIdResult> {
    for header_name in &["x-claude-code-session-id", "claude-code-session-id"] {
        if let Some(value) = headers.get(*header_name) {
            if let Ok(session_id) = value.to_str() {
                if !session_id.is_empty() {
                    return Some(SessionIdResult {
                        session_id: session_id.to_string(),
                        source: SessionIdSource::Header,
                        client_provided: true,
                    });
                }
            }
        }
    }

    extract_from_metadata(body)
}

/// 提取 Responses 客户端的 Session ID
fn extract_responses_session(
    headers: &HeaderMap,
    body: &serde_json::Value,
    prefix: &str,
) -> Option<SessionIdResult> {
    // 新版 Codex 将稳定会话身份放在 x-codex-turn-metadata JSON 头中。
    // 优先使用 thread_id/session_id，避免每个请求生成随机 ID 而无法匹配项目。
    if prefix == "codex" {
        if let Some(result) = extract_codex_turn_metadata(headers) {
            return Some(result);
        }
    }

    // 1. 从 headers 提取
    let header_names: &[&str] = if prefix == "grokbuild" {
        // Conversation ID 跨多轮请求保持稳定；session ID 作为客户端缺少
        // conversation ID 时的回退。x-grok-req-id 是逐请求 ID，不能用于聚合。
        &["x-grok-conv-id", "x-grok-session-id"]
    } else {
        &["session_id", "x-session-id"]
    };
    for header_name in header_names {
        if let Some(value) = headers.get(*header_name) {
            if let Ok(session_id) = value.to_str() {
                let session_id = session_id.trim();
                // Responses 客户端的 Session ID 通常较长（UUID 格式）
                if session_id.len() > 20 {
                    return Some(SessionIdResult {
                        session_id: format!("{prefix}_{session_id}"),
                        source: SessionIdSource::Header,
                        client_provided: true,
                    });
                }
            }
        }
    }

    // 2. 从 body.metadata.session_id 提取
    if let Some(session_id) = body
        .get("metadata")
        .and_then(|m| m.get("session_id"))
        .and_then(|v| v.as_str())
    {
        let session_id = session_id.trim();
        if session_id.len() > 10 {
            return Some(SessionIdResult {
                session_id: format!("{prefix}_{session_id}"),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    // previous_response_id 是 Responses 协议里的响应游标，不是稳定会话身份。
    // Chat/Responses 桥接时该值通常来自上游每轮返回的随机 response id；
    // 若把它当 prompt_cache_key 或 Codex session header，会导致每轮请求换缓存 key。

    None
}

/// 从 Codex 的 turn metadata JSON 请求头提取稳定会话 ID。
fn extract_codex_turn_metadata(headers: &HeaderMap) -> Option<SessionIdResult> {
    let metadata = parse_codex_turn_metadata(headers)?;
    let session_id = metadata
        .get("thread_id")
        .or_else(|| metadata.get("session_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    Some(SessionIdResult {
        session_id: format!("codex_{session_id}"),
        source: SessionIdSource::Header,
        client_provided: true,
    })
}

fn parse_codex_turn_metadata(headers: &HeaderMap) -> Option<serde_json::Value> {
    let value = headers.get("x-codex-turn-metadata")?.to_str().ok()?;
    serde_json::from_str::<serde_json::Value>(value).ok()
}

/// Extract workspace roots from Codex turn metadata.
///
/// New Codex versions include a `workspaces` object whose keys are normalized
/// workspace roots. This lets project routing work on the first request of a
/// session, before rollout history has been flushed and scanned.
pub(crate) fn extract_codex_workspace_paths(headers: &HeaderMap) -> Vec<String> {
    let Some(metadata) = parse_codex_turn_metadata(headers) else {
        return Vec::new();
    };

    let Some(workspaces) = metadata.get("workspaces") else {
        return Vec::new();
    };

    if let Some(workspaces) = workspaces.as_object() {
        return workspaces
            .keys()
            .map(|path| path.trim())
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect();
    }

    workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|workspace| {
            workspace
                .as_str()
                .or_else(|| workspace.get("path").and_then(|value| value.as_str()))
        })
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

/// 从 metadata 提取 Session ID (Claude)
fn extract_from_metadata(body: &serde_json::Value) -> Option<SessionIdResult> {
    let metadata = body.get("metadata")?;

    // 1. 从 metadata.user_id 提取（格式: user_xxx_session_yyy）
    if let Some(user_id) = metadata.get("user_id").and_then(|v| v.as_str()) {
        if let Some(session_id) = parse_session_from_user_id(user_id) {
            return Some(SessionIdResult {
                session_id,
                source: SessionIdSource::MetadataUserId,
                client_provided: true,
            });
        }
    }

    // 2. 直接从 metadata.session_id 提取
    if let Some(session_id) = metadata.get("session_id").and_then(|v| v.as_str()) {
        if !session_id.is_empty() {
            return Some(SessionIdResult {
                session_id: session_id.to_string(),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    None
}

/// 从 user_id 解析 session_id
///
/// 格式: `user_identifier_session_actual_session_id`
pub(super) fn parse_session_from_user_id(user_id: &str) -> Option<String> {
    // 查找 "_session_" 分隔符
    if let Some(pos) = user_id.find("_session_") {
        let session_id = &user_id[pos + 9..]; // "_session_" 长度为 9
        if !session_id.is_empty() {
            return Some(session_id.to_string());
        }
    }
    None
}

/// 生成新的 Session ID
fn generate_new_session_id() -> SessionIdResult {
    SessionIdResult {
        session_id: Uuid::new_v4().to_string(),
        source: SessionIdSource::Generated,
        client_provided: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========== Session ID 提取测试 ==========

    #[test]
    fn test_extract_session_from_claude_metadata_user_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {
                "user_id": "user_john_doe_session_abc123def456"
            }
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "abc123def456");
        assert_eq!(result.source, SessionIdSource::MetadataUserId);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_from_claude_metadata_session_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {
                "session_id": "my-session-123"
            }
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "my-session-123");
        assert_eq!(result.source, SessionIdSource::MetadataSessionId);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_from_claude_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-claude-code-session-id",
            "d937243f-2702-4f20-97b6-c9682235ab81".parse().unwrap(),
        );
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "d937243f-2702-4f20-97b6-c9682235ab81");
        assert_eq!(result.source, SessionIdSource::Header);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_from_claude_header_precedes_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-claude-code-session-id",
            "header-session-123".parse().unwrap(),
        );
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {
                "session_id": "my-session-123"
            }
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "header-session-123");
        assert_eq!(result.source, SessionIdSource::Header);
        assert!(result.client_provided);
    }

    #[test]
    fn test_codex_turn_metadata_prefers_thread_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-turn-metadata",
            r#"{"session_id":"session-fallback","thread_id":"01a0b87f-0573-7901-8154-99ba04747b6e","turn_id":"turn-1"}"#
                .parse()
                .unwrap(),
        );

        let result = extract_session_id(&headers, &serde_json::json!({}), "codex");

        assert_eq!(
            result.session_id,
            "codex_01a0b87f-0573-7901-8154-99ba04747b6e"
        );
        assert_eq!(result.source, SessionIdSource::Header);
        assert!(result.client_provided);
    }

    #[test]
    fn test_codex_turn_metadata_falls_back_to_session_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-turn-metadata",
            r#"{"session_id":"01a0b87f-0573-7901-8154-99ba04747b6e"}"#
                .parse()
                .unwrap(),
        );

        let result = extract_session_id(&headers, &serde_json::json!({}), "codex");

        assert_eq!(
            result.session_id,
            "codex_01a0b87f-0573-7901-8154-99ba04747b6e"
        );
        assert_eq!(result.source, SessionIdSource::Header);
        assert!(result.client_provided);
    }

    #[test]
    fn test_codex_turn_metadata_extracts_workspace_paths() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-turn-metadata",
            r#"{"thread_id":"thread-1","workspaces":{"C:\\project\\demo":{"has_changes":true},"C:\\project\\demo\\sub":{"has_changes":false}}}"#
                .parse()
                .unwrap(),
        );

        let paths = extract_codex_workspace_paths(&headers);

        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&r"C:\project\demo".to_string()));
        assert!(paths.contains(&r"C:\project\demo\sub".to_string()));
    }

    #[test]
    fn test_codex_previous_response_id_is_not_stable_session_identity() {
        let headers = HeaderMap::new();
        let body = json!({
            "input": "Write a function",
            "previous_response_id": "resp_abc123def456789"
        });

        let result = extract_session_id(&headers, &body, "codex");

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_codex_keeps_existing_response_session_headers() {
        let body = json!({ "input": "Write a function" });

        for header_name in ["session_id", "x-session-id"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header_name,
                "d937243f-2702-4f20-97b6-c9682235ab81".parse().unwrap(),
            );

            let result = extract_session_id(&headers, &body, "codex");

            assert_eq!(
                result.session_id,
                "codex_d937243f-2702-4f20-97b6-c9682235ab81"
            );
            assert_eq!(result.source, SessionIdSource::Header);
            assert!(result.client_provided);
        }
    }

    #[test]
    fn test_grokbuild_prefers_conversation_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-grok-conv-id",
            "conv-724f4275-584e-43af-ad46-b5e7509a3ca2".parse().unwrap(),
        );
        headers.insert(
            "x-grok-session-id",
            "session-d937243f-2702-4f20-97b6-c9682235ab81"
                .parse()
                .unwrap(),
        );
        let body = json!({ "input": "Write a function" });

        let result = extract_session_id(&headers, &body, "grokbuild");

        assert_eq!(
            result.session_id,
            "grokbuild_conv-724f4275-584e-43af-ad46-b5e7509a3ca2"
        );
        assert_eq!(result.source, SessionIdSource::Header);
        assert!(result.client_provided);
    }

    #[test]
    fn test_grokbuild_falls_back_to_session_header() {
        let body = json!({ "input": "Write a function" });

        for conversation_id in ["", "                         "] {
            let mut headers = HeaderMap::new();
            headers.insert("x-grok-conv-id", conversation_id.parse().unwrap());
            headers.insert(
                "x-grok-session-id",
                "session-d937243f-2702-4f20-97b6-c9682235ab81"
                    .parse()
                    .unwrap(),
            );

            let result = extract_session_id(&headers, &body, "grokbuild");

            assert_eq!(
                result.session_id,
                "grokbuild_session-d937243f-2702-4f20-97b6-c9682235ab81"
            );
            assert_eq!(result.source, SessionIdSource::Header);
            assert!(result.client_provided);
        }
    }

    #[test]
    fn test_grokbuild_ignores_request_and_codex_session_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-grok-req-id",
            "request-724f4275-584e-43af-ad46-b5e7509a3ca2"
                .parse()
                .unwrap(),
        );
        headers.insert(
            "x-session-id",
            "codex-d937243f-2702-4f20-97b6-c9682235ab81"
                .parse()
                .unwrap(),
        );
        let body = json!({ "input": "Write a function" });

        let result = extract_session_id(&headers, &body, "grokbuild");

        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_extract_session_generates_new_when_not_found() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_parse_session_from_user_id() {
        assert_eq!(
            parse_session_from_user_id("user_john_session_abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_session_from_user_id("my_app_session_xyz789"),
            Some("xyz789".to_string())
        );
        // 注意: "_session_" 是分隔符，所以下面的字符串会匹配
        assert_eq!(
            parse_session_from_user_id("no_session_marker"),
            Some("marker".to_string())
        );
        // 没有 "_session_" 分隔符的情况
        assert_eq!(parse_session_from_user_id("user_john_abc123"), None);
        assert_eq!(parse_session_from_user_id("_session_"), None);
    }
}

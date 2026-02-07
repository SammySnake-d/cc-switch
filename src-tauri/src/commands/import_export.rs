#![allow(non_snake_case)]

use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::services::provider::ProviderService;
use crate::store::AppState;

const DEFAULT_WEBDAV_FILE_NAME: &str = "cc-switch-backup.zip";
const WEBDAV_TIMEOUT_SECS: u64 = 45;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavTransferRequest {
    pub url: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub remote_dir: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone)]
struct PreparedWebDavRequest {
    target_url: Url,
    directory_urls: Vec<Url>,
    file_name: String,
    username: Option<String>,
    password: Option<String>,
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_base_url(raw: &str) -> Result<Url, AppError> {
    let mut base = Url::parse(raw.trim())
        .map_err(|e| AppError::InvalidInput(format!("WebDAV 地址无效: {e}")))?;

    if !matches!(base.scheme(), "http" | "https") {
        return Err(AppError::InvalidInput(
            "WebDAV 地址仅支持 http/https".to_string(),
        ));
    }

    if base.query().is_some() || base.fragment().is_some() {
        return Err(AppError::InvalidInput(
            "WebDAV 地址不应包含 query 或 fragment".to_string(),
        ));
    }

    let mut path = base.path().to_string();
    if !path.ends_with('/') {
        path.push('/');
        base.set_path(&path);
    }

    Ok(base)
}

fn parse_webdav_segments(raw: Option<&str>) -> Result<Vec<String>, AppError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };

    let normalized = raw.replace('\\', "/");
    let mut segments = Vec::new();

    for segment in normalized.split('/') {
        let part = segment.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(AppError::InvalidInput(
                "WebDAV 目录不允许包含 ..".to_string(),
            ));
        }
        segments.push(part.to_string());
    }

    Ok(segments)
}

fn normalize_file_name(raw: Option<String>) -> Result<String, AppError> {
    let file_name = normalize_optional(raw).unwrap_or_else(|| DEFAULT_WEBDAV_FILE_NAME.to_string());
    if file_name == "." || file_name == ".." || file_name.contains('/') || file_name.contains('\\')
    {
        return Err(AppError::InvalidInput(
            "WebDAV 文件名无效，请仅填写文件名（例如 backup.zip）".to_string(),
        ));
    }
    Ok(file_name)
}

fn build_webdav_target_url(
    base_url: &Url,
    directory_segments: &[String],
    file_name: &str,
) -> Result<Url, AppError> {
    let mut target = base_url.clone();
    {
        let mut path_segments = target.path_segments_mut().map_err(|_| {
            AppError::InvalidInput("WebDAV 地址无法拼接路径，请检查格式".to_string())
        })?;
        path_segments.pop_if_empty();
        for segment in directory_segments {
            path_segments.push(segment);
        }
        path_segments.push(file_name);
    }
    Ok(target)
}

fn build_webdav_directory_urls(
    base_url: &Url,
    directory_segments: &[String],
) -> Result<Vec<Url>, AppError> {
    let mut urls = Vec::with_capacity(directory_segments.len());
    for idx in 0..directory_segments.len() {
        let mut collection_url = base_url.clone();
        {
            let mut path_segments = collection_url.path_segments_mut().map_err(|_| {
                AppError::InvalidInput("WebDAV 地址无法拼接目录，请检查格式".to_string())
            })?;
            path_segments.pop_if_empty();
            for segment in &directory_segments[..=idx] {
                path_segments.push(segment);
            }
            path_segments.push("");
        }
        urls.push(collection_url);
    }
    Ok(urls)
}

fn prepare_webdav_request(
    request: WebDavTransferRequest,
) -> Result<PreparedWebDavRequest, AppError> {
    let trimmed_url = request.url.trim();
    if trimmed_url.is_empty() {
        return Err(AppError::InvalidInput("WebDAV 地址不能为空".to_string()));
    }

    let base_url = normalize_base_url(trimmed_url)?;
    let remote_dir = normalize_optional(request.remote_dir);
    let directory_segments = parse_webdav_segments(remote_dir.as_deref())?;
    let file_name = normalize_file_name(request.file_name)?;
    let target_url = build_webdav_target_url(&base_url, &directory_segments, &file_name)?;
    let directory_urls = build_webdav_directory_urls(&base_url, &directory_segments)?;

    Ok(PreparedWebDavRequest {
        target_url,
        directory_urls,
        file_name,
        username: normalize_optional(request.username),
        password: request.password.and_then(|pwd| {
            if pwd.trim().is_empty() {
                None
            } else {
                Some(pwd)
            }
        }),
    })
}

fn apply_webdav_auth(
    mut builder: reqwest::RequestBuilder,
    username: Option<&str>,
    password: Option<&str>,
) -> reqwest::RequestBuilder {
    if let Some(username) = username {
        builder = builder.basic_auth(username, Some(password.unwrap_or("")));
    }
    builder
}

fn format_http_error(method: &str, url: &Url, status: StatusCode, body_excerpt: &str) -> String {
    let reason = status.canonical_reason().unwrap_or("Unknown");
    if body_excerpt.is_empty() {
        format!("{method} {url} 失败: HTTP {} {reason}", status.as_u16())
    } else {
        format!(
            "{method} {url} 失败: HTTP {} {reason}; 响应: {body_excerpt}",
            status.as_u16()
        )
    }
}

async fn response_excerpt(response: reqwest::Response) -> String {
    let text = match response.text().await {
        Ok(text) => text,
        Err(_) => return String::new(),
    };

    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut iter = compact.chars();
    let excerpt: String = iter.by_ref().take(160).collect();
    if iter.next().is_some() {
        format!("{excerpt}...")
    } else {
        excerpt
    }
}

async fn check_collection_exists(
    client: &reqwest::Client,
    url: &Url,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<bool, AppError> {
    let method = Method::from_bytes(b"PROPFIND")
        .map_err(|e| AppError::Message(format!("初始化 PROPFIND 方法失败: {e}")))?;
    let request = client.request(method, url.clone()).header("Depth", "0");
    let request = apply_webdav_auth(request, username, password);
    let response = request
        .send()
        .await
        .map_err(|e| AppError::Message(format!("检查 WebDAV 目录失败: {e}")))?;

    let status = response.status();
    Ok(status.is_success() || status.as_u16() == 207)
}

async fn ensure_webdav_directories(
    client: &reqwest::Client,
    prepared: &PreparedWebDavRequest,
) -> Result<(), AppError> {
    if prepared.directory_urls.is_empty() {
        return Ok(());
    }

    let method = Method::from_bytes(b"MKCOL")
        .map_err(|e| AppError::Message(format!("初始化 MKCOL 方法失败: {e}")))?;

    for collection_url in &prepared.directory_urls {
        let request = client.request(method.clone(), collection_url.clone());
        let request = apply_webdav_auth(
            request,
            prepared.username.as_deref(),
            prepared.password.as_deref(),
        );
        let response = request
            .send()
            .await
            .map_err(|e| AppError::Message(format!("创建 WebDAV 目录失败: {e}")))?;
        let status = response.status();
        if status.is_success() || matches!(status.as_u16(), 200 | 204 | 301 | 302 | 405) {
            continue;
        }

        if matches!(status.as_u16(), 403 | 409)
            && check_collection_exists(
                client,
                collection_url,
                prepared.username.as_deref(),
                prepared.password.as_deref(),
            )
            .await
            .unwrap_or(false)
        {
            continue;
        }

        let body_excerpt = response_excerpt(response).await;
        return Err(AppError::Message(format_http_error(
            "MKCOL",
            collection_url,
            status,
            &body_excerpt,
        )));
    }

    Ok(())
}

/// 导出数据库为 SQL 备份
#[tauri::command]
pub async fn export_config_to_file(
    #[allow(non_snake_case)] filePath: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target_path = PathBuf::from(&filePath);
        db.export_sql(&target_path)?;
        Ok::<_, AppError>(json!({
            "success": true,
            "message": "SQL exported successfully",
            "filePath": filePath
        }))
    })
    .await
    .map_err(|e| format!("导出配置失败: {e}"))?
    .map_err(|e: AppError| e.to_string())
}

/// 从 SQL 备份导入数据库
#[tauri::command]
pub async fn import_config_from_file(
    #[allow(non_snake_case)] filePath: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let db = state.db.clone();
    let db_for_state = db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path_buf = PathBuf::from(&filePath);
        let backup_id = db.import_sql(&path_buf)?;

        // 导入后同步当前供应商到各自的 live 配置
        let app_state = AppState::new(db_for_state);
        if let Err(err) = ProviderService::sync_current_to_live(&app_state) {
            log::warn!("导入后同步 live 配置失败: {err}");
        }

        // 重新加载设置到内存缓存，确保导入的设置生效
        if let Err(err) = crate::settings::reload_settings() {
            log::warn!("导入后重载设置失败: {err}");
        }

        Ok::<_, AppError>(json!({
            "success": true,
            "message": "SQL imported successfully",
            "backupId": backup_id
        }))
    })
    .await
    .map_err(|e| format!("导入配置失败: {e}"))?
    .map_err(|e: AppError| e.to_string())
}

#[tauri::command]
pub async fn sync_current_providers_live(state: State<'_, AppState>) -> Result<Value, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app_state = AppState::new(db);
        ProviderService::sync_current_to_live(&app_state)?;
        Ok::<_, AppError>(json!({
            "success": true,
            "message": "Live configuration synchronized"
        }))
    })
    .await
    .map_err(|e| format!("同步当前供应商失败: {e}"))?
    .map_err(|e: AppError| e.to_string())
}

/// 保存文件对话框
#[tauri::command]
pub async fn save_file_dialog<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    #[allow(non_snake_case)] defaultName: String,
) -> Result<Option<String>, String> {
    let dialog = app.dialog();
    let result = dialog
        .file()
        .add_filter("SQL", &["sql"])
        .set_file_name(&defaultName)
        .blocking_save_file();

    Ok(result.map(|p| p.to_string()))
}

/// 打开文件对话框
#[tauri::command]
pub async fn open_file_dialog<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    let dialog = app.dialog();
    let result = dialog
        .file()
        .add_filter("SQL", &["sql"])
        .blocking_pick_file();

    Ok(result.map(|p| p.to_string()))
}

/// 打开 ZIP 文件选择对话框
#[tauri::command]
pub async fn open_zip_file_dialog<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    let dialog = app.dialog();
    let result = dialog
        .file()
        .add_filter("ZIP", &["zip"])
        .blocking_pick_file();

    Ok(result.map(|p| p.to_string()))
}

/// 上传全量备份到 WebDAV
#[tauri::command]
pub async fn upload_config_backup_to_webdav(
    request: WebDavTransferRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let prepared = prepare_webdav_request(request).map_err(|e| e.to_string())?;
    let db = state.db.clone();

    let backup_bytes = tauri::async_runtime::spawn_blocking(move || {
        crate::backup_bundle::build_full_backup_archive(&db)
    })
    .await
    .map_err(|e| format!("构建全量备份失败: {e}"))?
    .map_err(|e: AppError| e.to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(WEBDAV_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("初始化 WebDAV 客户端失败: {e}"))?;

    ensure_webdav_directories(&client, &prepared)
        .await
        .map_err(|e| e.to_string())?;

    let request = client
        .put(prepared.target_url.clone())
        .header("Content-Type", "application/zip")
        .body(backup_bytes);
    let request = apply_webdav_auth(
        request,
        prepared.username.as_deref(),
        prepared.password.as_deref(),
    );
    let response = request
        .send()
        .await
        .map_err(|e| format!("上传 WebDAV 备份失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body_excerpt = response_excerpt(response).await;
        return Err(format_http_error(
            "PUT",
            &prepared.target_url,
            status,
            &body_excerpt,
        ));
    }

    Ok(json!({
        "success": true,
        "message": "Full backup uploaded to WebDAV",
        "fileName": prepared.file_name,
        "remoteUrl": prepared.target_url.to_string()
    }))
}

/// 从 WebDAV 下载备份并恢复（支持全量 ZIP 或旧版 SQL）
#[tauri::command]
pub async fn download_config_backup_from_webdav(
    request: WebDavTransferRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let prepared = prepare_webdav_request(request).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(WEBDAV_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("初始化 WebDAV 客户端失败: {e}"))?;

    let request = apply_webdav_auth(
        client.get(prepared.target_url.clone()),
        prepared.username.as_deref(),
        prepared.password.as_deref(),
    );
    let response = request
        .send()
        .await
        .map_err(|e| format!("下载 WebDAV 备份失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body_excerpt = response_excerpt(response).await;
        return Err(format_http_error(
            "GET",
            &prepared.target_url,
            status,
            &body_excerpt,
        ));
    }

    let backup_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("读取 WebDAV 响应失败: {e}"))?
        .to_vec();

    if backup_bytes.is_empty() {
        return Err("WebDAV 备份文件为空".to_string());
    }

    let db = state.db.clone();
    let restore_result = tauri::async_runtime::spawn_blocking(move || {
        crate::backup_bundle::restore_backup_from_bytes(&db, &backup_bytes)
    })
    .await
    .map_err(|e| format!("恢复 WebDAV 备份失败: {e}"))?
    .map_err(|e: AppError| e.to_string())?;

    let message = if restore_result.full_restore {
        "Full backup restored successfully"
    } else {
        "SQL backup imported successfully"
    };

    Ok(json!({
        "success": true,
        "message": message,
        "backupId": restore_result.backup_id,
        "fullRestore": restore_result.full_restore,
        "fileName": prepared.file_name,
        "remoteUrl": prepared.target_url.to_string()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webdav_segments_normalizes_separators() {
        let segments = parse_webdav_segments(Some("/foo\\bar//baz/")).expect("parse segments");
        assert_eq!(segments, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn parse_webdav_segments_rejects_parent_segment() {
        let err = parse_webdav_segments(Some("foo/../bar")).expect_err("reject ..");
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn prepare_webdav_request_builds_target_url() {
        let request = WebDavTransferRequest {
            url: "https://dav.example.com/remote.php/dav/files/user".to_string(),
            username: None,
            password: None,
            remote_dir: Some("/cc-switch/backups/".to_string()),
            file_name: Some("daily.zip".to_string()),
        };

        let prepared = prepare_webdav_request(request).expect("prepare request");
        assert_eq!(
            prepared.target_url.as_str(),
            "https://dav.example.com/remote.php/dav/files/user/cc-switch/backups/daily.zip"
        );
    }

    #[test]
    fn prepare_webdav_request_uses_default_file_name() {
        let request = WebDavTransferRequest {
            url: "https://dav.example.com/webdav".to_string(),
            username: None,
            password: None,
            remote_dir: None,
            file_name: None,
        };

        let prepared = prepare_webdav_request(request).expect("prepare request");
        assert_eq!(prepared.file_name, DEFAULT_WEBDAV_FILE_NAME);
    }
}

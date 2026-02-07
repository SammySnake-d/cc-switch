use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::app_config::AppType;
use crate::error::AppError;
use crate::prompt_files::prompt_file_path;
use crate::services::provider::ProviderService;
use crate::services::skill::SkillService;
use crate::store::AppState;
use crate::Database;

const BACKUP_FORMAT: &str = "cc-switch-full-backup";
const BACKUP_VERSION: u32 = 1;
const MANIFEST_ENTRY: &str = "cc-switch-backup/manifest.json";
const DB_SQL_ENTRY: &str = "cc-switch-backup/db/export.sql";
const SETTINGS_ENTRY: &str = "cc-switch-backup/app/settings.json";
const LEGACY_CONFIG_ENTRY: &str = "cc-switch-backup/app/config.json";
const SKILLS_PREFIX: &str = "cc-switch-backup/app/skills";

const CLAUDE_SETTINGS_ENTRY: &str = "cc-switch-backup/system/claude/settings.json";
const CLAUDE_MCP_ENTRY: &str = "cc-switch-backup/system/claude/mcp.json";
const CODEX_AUTH_ENTRY: &str = "cc-switch-backup/system/codex/auth.json";
const CODEX_CONFIG_ENTRY: &str = "cc-switch-backup/system/codex/config.toml";
const GEMINI_ENV_ENTRY: &str = "cc-switch-backup/system/gemini/.env";
const GEMINI_SETTINGS_ENTRY: &str = "cc-switch-backup/system/gemini/settings.json";
const OPENCODE_CONFIG_ENTRY: &str = "cc-switch-backup/system/opencode/opencode.json";
const OPENCODE_ENV_ENTRY: &str = "cc-switch-backup/system/opencode/.env";

#[derive(Debug, Clone)]
pub struct RestoreResult {
    pub backup_id: String,
    pub full_restore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format: String,
    version: u32,
    created_at: String,
}

pub fn build_full_backup_archive(db: &Arc<Database>) -> Result<Vec<u8>, AppError> {
    let sql_bytes = export_sql_to_bytes(db)?;

    let mut writer = ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    add_bytes_entry(&mut writer, DB_SQL_ENTRY, &sql_bytes)?;

    let settings = crate::settings::get_settings();
    let settings_bytes =
        serde_json::to_vec_pretty(&settings).map_err(|e| AppError::JsonSerialize { source: e })?;
    add_bytes_entry(&mut writer, SETTINGS_ENTRY, &settings_bytes)?;

    let _ = add_file_if_exists(
        &mut writer,
        LEGACY_CONFIG_ENTRY,
        &crate::config::get_app_config_path(),
    )?;

    if let Ok(skills_dir) = SkillService::get_ssot_dir() {
        let _ = add_directory_recursive_if_exists(&mut writer, SKILLS_PREFIX, &skills_dir)?;
    }

    let _ = add_file_if_exists(
        &mut writer,
        CLAUDE_SETTINGS_ENTRY,
        &crate::config::get_claude_settings_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        CLAUDE_MCP_ENTRY,
        &crate::config::get_claude_mcp_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        CODEX_AUTH_ENTRY,
        &crate::codex_config::get_codex_auth_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        CODEX_CONFIG_ENTRY,
        &crate::codex_config::get_codex_config_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        GEMINI_ENV_ENTRY,
        &crate::gemini_config::get_gemini_env_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        GEMINI_SETTINGS_ENTRY,
        &crate::gemini_config::get_gemini_settings_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        OPENCODE_CONFIG_ENTRY,
        &crate::opencode_config::get_opencode_config_path(),
    )?;
    let _ = add_file_if_exists(
        &mut writer,
        OPENCODE_ENV_ENTRY,
        &crate::opencode_config::get_opencode_env_path(),
    )?;

    for app in AppType::all() {
        let Ok(path) = prompt_file_path(&app) else {
            continue;
        };
        let _ = add_file_if_exists(&mut writer, prompt_entry_for_app(&app), &path)?;
    }

    let manifest = BackupManifest {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_VERSION,
        created_at: Utc::now().to_rfc3339(),
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| AppError::JsonSerialize { source: e })?;
    add_bytes_entry(&mut writer, MANIFEST_ENTRY, &manifest_bytes)?;

    let cursor = writer
        .finish()
        .map_err(|e| AppError::Message(format!("完成备份 ZIP 失败: {e}")))?;
    Ok(cursor.into_inner())
}

pub fn restore_backup_from_bytes(
    db: &Arc<Database>,
    bytes: &[u8],
) -> Result<RestoreResult, AppError> {
    if looks_like_zip(bytes) {
        return restore_full_backup_archive(db, bytes);
    }

    let backup_id = import_sql_from_bytes(db, bytes)?;
    finalize_restore(db);
    Ok(RestoreResult {
        backup_id,
        full_restore: false,
    })
}

fn restore_full_backup_archive(
    db: &Arc<Database>,
    bytes: &[u8],
) -> Result<RestoreResult, AppError> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| AppError::Message(format!("解析备份 ZIP 失败: {e}")))?;

    let manifest_bytes = read_zip_entry_bytes(&mut archive, MANIFEST_ENTRY)?.ok_or_else(|| {
        AppError::Message("备份包缺少 manifest.json，无法识别为 CC Switch 全量备份".to_string())
    })?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        AppError::Message(format!("解析备份 manifest.json 失败（JSON 格式无效）: {e}"))
    })?;

    if manifest.format != BACKUP_FORMAT {
        return Err(AppError::Message(format!(
            "备份包格式不匹配: {}",
            manifest.format
        )));
    }
    if manifest.version != BACKUP_VERSION {
        return Err(AppError::Message(format!(
            "备份包版本不支持: {}",
            manifest.version
        )));
    }

    let sql_bytes = read_zip_entry_bytes(&mut archive, DB_SQL_ENTRY)?.ok_or_else(|| {
        AppError::Message("备份包缺少数据库 SQL 文件（db/export.sql）".to_string())
    })?;
    let backup_id = import_sql_from_bytes(db, &sql_bytes)?;

    if let Some(settings_bytes) = read_zip_entry_bytes(&mut archive, SETTINGS_ENTRY)? {
        let settings: crate::settings::AppSettings = serde_json::from_slice(&settings_bytes)
            .map_err(|e| AppError::Message(format!("解析 settings.json 失败: {e}")))?;
        crate::settings::update_settings(settings)?;
    }

    write_entry_to_path_if_present(
        &mut archive,
        LEGACY_CONFIG_ENTRY,
        &crate::config::get_app_config_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        CLAUDE_SETTINGS_ENTRY,
        &crate::config::get_claude_settings_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        CLAUDE_MCP_ENTRY,
        &crate::config::get_claude_mcp_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        CODEX_AUTH_ENTRY,
        &crate::codex_config::get_codex_auth_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        CODEX_CONFIG_ENTRY,
        &crate::codex_config::get_codex_config_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        GEMINI_ENV_ENTRY,
        &crate::gemini_config::get_gemini_env_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        GEMINI_SETTINGS_ENTRY,
        &crate::gemini_config::get_gemini_settings_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        OPENCODE_CONFIG_ENTRY,
        &crate::opencode_config::get_opencode_config_path(),
    )?;
    write_entry_to_path_if_present(
        &mut archive,
        OPENCODE_ENV_ENTRY,
        &crate::opencode_config::get_opencode_env_path(),
    )?;

    for app in AppType::all() {
        let Ok(path) = prompt_file_path(&app) else {
            continue;
        };
        write_entry_to_path_if_present(&mut archive, prompt_entry_for_app(&app), &path)?;
    }

    replace_skills_ssot_from_archive(&mut archive)?;
    finalize_restore(db);

    Ok(RestoreResult {
        backup_id,
        full_restore: true,
    })
}

fn export_sql_to_bytes(db: &Arc<Database>) -> Result<Vec<u8>, AppError> {
    let temp_file = tempfile::Builder::new()
        .prefix("cc-switch-full-backup-export-")
        .suffix(".sql")
        .tempfile()
        .map_err(|e| AppError::IoContext {
            context: "创建临时 SQL 文件失败".to_string(),
            source: e,
        })?;
    let temp_path = temp_file.path().to_path_buf();
    db.export_sql(&temp_path)?;
    fs::read(&temp_path).map_err(|e| AppError::io(&temp_path, e))
}

fn import_sql_from_bytes(db: &Arc<Database>, sql_bytes: &[u8]) -> Result<String, AppError> {
    let temp_file = tempfile::Builder::new()
        .prefix("cc-switch-full-backup-import-")
        .suffix(".sql")
        .tempfile()
        .map_err(|e| AppError::IoContext {
            context: "创建临时 SQL 文件失败".to_string(),
            source: e,
        })?;
    let temp_path = temp_file.path().to_path_buf();
    fs::write(&temp_path, sql_bytes).map_err(|e| AppError::io(&temp_path, e))?;
    db.import_sql(&temp_path)
}

fn finalize_restore(db: &Arc<Database>) {
    let app_state = AppState::new(db.clone());
    if let Err(err) = ProviderService::sync_current_to_live(&app_state) {
        log::warn!("恢复备份后同步 live 配置失败: {err}");
    }

    for app in AppType::all() {
        if let Err(err) = SkillService::sync_to_app(db, &app) {
            log::warn!("恢复备份后同步 Skill 到 {:?} 失败: {err:#}", app);
        }
    }

    if let Err(err) = crate::settings::reload_settings() {
        log::warn!("恢复备份后重载设置失败: {err}");
    }
}

fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && bytes[0] == b'P'
        && bytes[1] == b'K'
        && matches!(bytes[2], 3 | 5 | 7)
        && matches!(bytes[3], 4 | 6 | 8)
}

fn add_bytes_entry<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    entry_path: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    writer
        .start_file(entry_path, options)
        .map_err(|e| AppError::Message(format!("写入 ZIP 条目失败 ({entry_path}): {e}")))?;
    writer
        .write_all(bytes)
        .map_err(|e| AppError::Message(format!("写入 ZIP 数据失败 ({entry_path}): {e}")))?;
    Ok(())
}

fn add_file_if_exists<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    entry_path: &str,
    source_path: &Path,
) -> Result<bool, AppError> {
    if !source_path.exists() || !source_path.is_file() {
        return Ok(false);
    }

    let bytes = fs::read(source_path).map_err(|e| AppError::io(source_path, e))?;
    add_bytes_entry(writer, entry_path, &bytes)?;
    Ok(true)
}

fn add_directory_recursive_if_exists<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    entry_prefix: &str,
    source_dir: &Path,
) -> Result<bool, AppError> {
    if !source_dir.exists() || !source_dir.is_dir() {
        return Ok(false);
    }

    let dir_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    writer
        .add_directory(format!("{entry_prefix}/"), dir_options)
        .map_err(|e| AppError::Message(format!("创建 ZIP 目录失败 ({entry_prefix}): {e}")))?;

    let mut found_any = false;
    let mut stack = vec![source_dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).map_err(|e| AppError::io(&current, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| AppError::io(&current, e))?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|e| AppError::io(&path, e))?;

            if file_type.is_symlink() {
                log::warn!("跳过符号链接文件: {}", path.display());
                continue;
            }

            let rel = path
                .strip_prefix(source_dir)
                .map_err(|e| AppError::Message(format!("生成相对路径失败: {e}")))?;
            let rel_zip = rel.to_string_lossy().replace('\\', "/");
            let zip_path = format!("{entry_prefix}/{rel_zip}");

            if file_type.is_dir() {
                writer
                    .add_directory(format!("{zip_path}/"), dir_options)
                    .map_err(|e| {
                        AppError::Message(format!("创建 ZIP 目录失败 ({zip_path}): {e}"))
                    })?;
                stack.push(path);
                continue;
            }

            if file_type.is_file() {
                let bytes = fs::read(&path).map_err(|e| AppError::io(&path, e))?;
                add_bytes_entry(writer, &zip_path, &bytes)?;
                found_any = true;
            }
        }
    }

    Ok(found_any)
}

fn read_zip_entry_bytes<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entry_path: &str,
) -> Result<Option<Vec<u8>>, AppError> {
    match archive.by_name(entry_path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|e| AppError::Message(format!("读取备份条目失败 ({entry_path}): {e}")))?;
            Ok(Some(bytes))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(AppError::Message(format!(
            "访问备份条目失败 ({entry_path}): {e}"
        ))),
    }
}

fn write_entry_to_path_if_present<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entry_path: &str,
    target_path: &Path,
) -> Result<bool, AppError> {
    let Some(bytes) = read_zip_entry_bytes(archive, entry_path)? else {
        return Ok(false);
    };
    write_bytes_to_path(target_path, &bytes)?;
    Ok(true)
}

fn write_bytes_to_path(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    crate::config::atomic_write(path, bytes)
}

fn replace_skills_ssot_from_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<(), AppError> {
    let temp_root = tempfile::tempdir().map_err(|e| AppError::IoContext {
        context: "创建临时 skills 目录失败".to_string(),
        source: e,
    })?;
    let temp_skills = temp_root.path().join("skills");
    fs::create_dir_all(&temp_skills).map_err(|e| AppError::io(&temp_skills, e))?;

    let prefix = format!("{SKILLS_PREFIX}/");
    let mut extracted_any = false;
    for idx in 0..archive.len() {
        let mut file = archive
            .by_index(idx)
            .map_err(|e| AppError::Message(format!("读取备份 ZIP 索引失败: {e}")))?;

        let Some(enclosed) = file.enclosed_name() else {
            continue;
        };

        let zip_name = enclosed.to_string_lossy().replace('\\', "/");
        if !zip_name.starts_with(&prefix) {
            continue;
        }

        let rel = zip_name
            .strip_prefix(&prefix)
            .ok_or_else(|| AppError::Message("解析 skills 备份路径失败".to_string()))?;

        if rel.is_empty() {
            continue;
        }

        let out_path = temp_skills.join(rel);
        if file.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| AppError::io(&out_path, e))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }
        let mut out = fs::File::create(&out_path).map_err(|e| AppError::io(&out_path, e))?;
        std::io::copy(&mut file, &mut out).map_err(|e| AppError::IoContext {
            context: format!("写入 skills 文件失败: {}", out_path.display()),
            source: e,
        })?;
        extracted_any = true;
    }

    let target_dir = SkillService::get_ssot_dir()
        .map_err(|e| AppError::Message(format!("获取 skills SSOT 目录失败: {e:#}")))?;
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir).map_err(|e| AppError::io(&target_dir, e))?;
    }
    fs::create_dir_all(&target_dir).map_err(|e| AppError::io(&target_dir, e))?;

    if extracted_any {
        copy_dir_recursive(&temp_skills, &target_dir)?;
    }

    Ok(())
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), AppError> {
    if !from.exists() {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|e| AppError::io(to, e))?;

    let entries = fs::read_dir(from).map_err(|e| AppError::io(from, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::io(from, e))?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| AppError::io(&source_path, e))?;

        if file_type.is_symlink() {
            log::warn!("跳过符号链接文件: {}", source_path.display());
            continue;
        }
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            fs::copy(&source_path, &target_path).map_err(|e| AppError::IoContext {
                context: format!(
                    "复制目录文件失败 ({} -> {})",
                    source_path.display(),
                    target_path.display()
                ),
                source: e,
            })?;
        }
    }

    Ok(())
}

fn prompt_entry_for_app(app: &AppType) -> &'static str {
    match app {
        AppType::Claude => "cc-switch-backup/system/prompts/claude.md",
        AppType::Codex => "cc-switch-backup/system/prompts/codex.md",
        AppType::Gemini => "cc-switch-backup/system/prompts/gemini.md",
        AppType::OpenCode => "cc-switch-backup/system/prompts/opencode.md",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_magic_detection_works() {
        assert!(looks_like_zip(b"PK\x03\x04rest"));
        assert!(!looks_like_zip(b"not-zip"));
    }

    #[test]
    fn prompt_entry_mapping_is_stable() {
        assert_eq!(
            prompt_entry_for_app(&AppType::Codex),
            "cc-switch-backup/system/prompts/codex.md"
        );
        assert_eq!(
            prompt_entry_for_app(&AppType::OpenCode),
            "cc-switch-backup/system/prompts/opencode.md"
        );
    }

    #[test]
    fn backup_constants_are_under_root() {
        let root = "cc-switch-backup";
        assert!(MANIFEST_ENTRY.starts_with(root));
        assert!(DB_SQL_ENTRY.starts_with(root));
        assert!(SETTINGS_ENTRY.starts_with(root));
    }
}

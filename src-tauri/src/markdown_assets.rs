use crate::{commands::AppState, model::*, workspace};
use base64::{engine::general_purpose::STANDARD, Engine};
use clipboard_rs::{common::RustImage, Clipboard, ClipboardContext};
use image::{GenericImageView, ImageFormat};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, collections::HashSet, fs, path::PathBuf};
use tauri::State;

const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const FIELDS: [&str; 4] = ["actual", "expected", "preconditions", "steps"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownImageImport {
    draft: Value,
    asset: MarkdownAsset,
    preview_data_url: String,
}

struct ClipboardImage {
    bytes: Vec<u8>,
    name: String,
}

fn valid_field(field: &str) -> Result<()> {
    if FIELDS.contains(&field) {
        Ok(())
    } else {
        Err("该字段不支持 Markdown 图片".into())
    }
}

fn read_clipboard_image() -> Result<ClipboardImage> {
    let clipboard = ClipboardContext::new().map_err(|_| "无法读取系统剪贴板")?;
    let files = clipboard.get_files().unwrap_or_default();
    if files.len() > 1 {
        return Err("一次只能从系统文件管理器粘贴一张图片".into());
    }
    if let Some(path) = files.first() {
        let path = PathBuf::from(path);
        let metadata = fs::metadata(&path).map_err(|_| "剪贴板中的图片文件无法读取")?;
        if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
            return Err("剪贴板图片无效或超过 15 MB".into());
        }
        let bytes = fs::read(&path).map_err(|_| "剪贴板中的图片文件无法读取")?;
        return Ok(ClipboardImage {
            bytes,
            name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("pasted-image")
                .to_owned(),
        });
    }
    let image = clipboard
        .get_image()
        .map_err(|_| "剪贴板中没有可粘贴的单张图片")?;
    let png = image.to_png().map_err(|_| "剪贴板图片无法转换为 PNG")?;
    Ok(ClipboardImage {
        bytes: png.get_bytes().to_vec(),
        name: "pasted-image.png".into(),
    })
}

fn inspect_image(bytes: &[u8]) -> Result<(String, &'static str, u32, u32)> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片为空或超过 15 MB".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "仅支持 PNG 或 JPEG 图片")?;
    let (mime, extension) = match format {
        ImageFormat::Png => ("image/png", "png"),
        ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        _ => return Err("首版 Markdown 图片仅支持 PNG 或 JPEG".into()),
    };
    let decoded =
        image::load_from_memory_with_format(bytes, format).map_err(|_| "图片内容损坏，无法预览")?;
    let (width, height) = decoded.dimensions();
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("图片尺寸过大，最大边长 16384 像素".into());
    }
    Ok((mime.into(), extension, width, height))
}

fn safe_display_name(name: &str, extension: &str) -> String {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("pasted-image")
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || ['-', '_', ' '].contains(&character) {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    let stem = stem.trim().trim_matches('.');
    format!(
        "{}.{}",
        if stem.is_empty() {
            "pasted-image"
        } else {
            stem
        },
        extension
    )
}

fn preview_data_url(asset: &MarkdownAsset, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", asset.mime, STANDARD.encode(bytes))
}

fn ensure_scope(state: &AppState, id: &str) -> Result<()> {
    let scope = workspace::current_scope(state, "")?;
    if state.store.get::<String>("draft_scope", id)?.as_deref() != Some(&scope) {
        return Err("Markdown 图片不属于当前账号或项目".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn import_markdown_image(
    id: String,
    field: String,
    name: Option<String>,
    data_base64: Option<String>,
    state: State<'_, AppState>,
) -> Result<MarkdownImageImport> {
    valid_field(&field)?;
    uuid::Uuid::parse_str(&id).map_err(|_| "草稿 ID 格式不正确")?;
    state.editable(&id)?;
    ensure_scope(&state, &id)?;
    let clipboard_image = match data_base64 {
        Some(encoded) => {
            if encoded.len() > MAX_IMAGE_BYTES * 2 {
                return Err("图片超过 15 MB".into());
            }
            ClipboardImage {
                bytes: STANDARD
                    .decode(encoded)
                    .map_err(|_| "粘贴图片数据无法读取")?,
                name: name.unwrap_or_else(|| "pasted-image".into()),
            }
        }
        None => tauri::async_runtime::spawn_blocking(read_clipboard_image)
            .await
            .map_err(|_| "读取系统剪贴板失败".to_string())??,
    };
    let (mime, extension, width, height) = inspect_image(&clipboard_image.bytes)?;
    let sha256 = hex::encode(Sha256::digest(&clipboard_image.bytes));

    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    ensure_scope(&state, &id)?;
    let mut draft = state.store.draft(&id)?;
    let existing = markdown_assets(&draft)?
        .into_iter()
        .find(|asset| asset.sha256 == sha256 && PathBuf::from(&asset.path).is_file());
    let asset = if let Some(asset) = existing {
        asset
    } else {
        let asset_id = uuid();
        let directory = state
            .store
            .root
            .join("markdown-assets")
            .join(&id)
            .join(&asset_id);
        fs::create_dir_all(&directory).map_err(|_| "无法建立 Markdown 图片目录")?;
        let path = directory.join(format!("image.{extension}"));
        fs::write(&path, &clipboard_image.bytes).map_err(|_| "无法保存 Markdown 图片")?;
        let asset = MarkdownAsset {
            id: asset_id,
            field: field.clone(),
            name: safe_display_name(&clipboard_image.name, extension),
            path: path.to_string_lossy().into_owned(),
            mime,
            size: clipboard_image.bytes.len() as u64,
            sha256,
            width,
            height,
            state: "staged".into(),
            remote_id: None,
            embed_markdown: String::new(),
            embed_url: String::new(),
            error: String::new(),
        };
        draft["markdownAssets"]
            .as_array_mut()
            .ok_or("Markdown 图片资源列表格式错误")?
            .push(serde_json::to_value(&asset).map_err(|e| e.to_string())?);
        asset
    };
    draft["previewConfirmed"] = json!(false);
    draft["updatedAt"] = json!(now());
    crate::validation::check_shape(&draft)?;
    state.assert_no_token(&draft)?;
    state.store.put("draft", &id, &draft)?;
    Ok(MarkdownImageImport {
        draft,
        preview_data_url: preview_data_url(&asset, &clipboard_image.bytes),
        asset,
    })
}

#[tauri::command]
pub fn read_markdown_asset(
    id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<String> {
    uuid::Uuid::parse_str(&id).map_err(|_| "草稿 ID 格式不正确")?;
    uuid::Uuid::parse_str(&asset_id).map_err(|_| "Markdown 图片 ID 格式不正确")?;
    ensure_scope(&state, &id)?;
    let draft = state.store.draft(&id)?;
    let asset = markdown_assets(&draft)?
        .into_iter()
        .find(|asset| asset.id == asset_id)
        .ok_or("Markdown 图片不存在")?;
    verify_asset_path(&state.store.root, &id, &asset)?;
    let bytes = fs::read(&asset.path).map_err(|_| "Markdown 图片无法读取")?;
    if bytes.len() as u64 != asset.size || hex::encode(Sha256::digest(&bytes)) != asset.sha256 {
        return Err("Markdown 图片完整性校验失败".into());
    }
    Ok(preview_data_url(&asset, &bytes))
}

pub fn referenced_ids(draft: &Value) -> HashSet<String> {
    let expression = regex::Regex::new(r"abd-asset://([0-9a-fA-F-]{36})").unwrap();
    let mut ids = HashSet::new();
    for field in FIELDS {
        let value = string(draft, field);
        ids.extend(
            expression
                .captures_iter(&value)
                .filter_map(|captures| captures.get(1).map(|item| item.as_str().to_owned())),
        );
    }
    ids
}

pub fn snapshot(root: &std::path::Path, draft: &Value) -> Result<Vec<MarkdownAsset>> {
    let referenced = referenced_ids(draft);
    let mut assets = HashMap::new();
    for asset in markdown_assets(draft)? {
        if assets.insert(asset.id.clone(), asset).is_some() {
            return Err("Markdown 图片资源 ID 重复".into());
        }
    }
    let mut result = Vec::new();
    for id in referenced {
        uuid::Uuid::parse_str(&id).map_err(|_| "Markdown 图片资源 ID 格式错误")?;
        let asset = assets
            .remove(&id)
            .ok_or_else(|| format!("Markdown 图片 {id} 的本地资源不存在"))?;
        verify_integrity(root, &string(draft, "id"), &asset)?;
        result.push(asset);
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(result)
}

pub fn verify_integrity(
    root: &std::path::Path,
    draft_id: &str,
    asset: &MarkdownAsset,
) -> Result<()> {
    verify_asset_path(root, draft_id, asset)?;
    let bytes = fs::read(&asset.path).map_err(|_| "Markdown 图片无法读取")?;
    if bytes.len() as u64 != asset.size || hex::encode(Sha256::digest(&bytes)) != asset.sha256 {
        return Err(format!("Markdown 图片 {} 已被修改，请重新粘贴", asset.name));
    }
    Ok(())
}

fn replace_references<F>(markdown: &str, assets: &[MarkdownAsset], resolver: F) -> Result<String>
where
    F: Fn(&MarkdownAsset) -> Result<String>,
{
    let by_id = assets
        .iter()
        .map(|asset| (asset.id.as_str(), asset))
        .collect::<HashMap<_, _>>();
    let expression = regex::Regex::new(
        r#"!\[(?:\\.|[^\]])*\]\(abd-asset://([0-9a-fA-F-]{36})(?:\s+"(?:\\.|[^"])*")?\)"#,
    )
    .unwrap();
    let mut error = None;
    let rendered = expression
        .replace_all(markdown, |captures: &regex::Captures<'_>| {
            let id = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            match by_id
                .get(id)
                .copied()
                .ok_or_else(|| format!("Markdown 图片 {id} 的资源索引不存在，不能生成提交内容"))
                .and_then(&resolver)
            {
                Ok(value) => value,
                Err(reason) => {
                    error = Some(reason);
                    captures.get(0).unwrap().as_str().to_owned()
                }
            }
        })
        .into_owned();
    if let Some(reason) = error {
        return Err(reason);
    }
    if rendered.contains("abd-asset://") {
        return Err("Markdown 中存在无法识别的图片引用，请删除后重新粘贴".into());
    }
    Ok(rendered)
}

pub fn render_remote(markdown: &str, assets: &[MarkdownAsset]) -> Result<String> {
    replace_references(markdown, assets, |asset| {
        if asset.state != "uploaded" || asset.embed_markdown.trim().is_empty() {
            return Err(format!("Markdown 图片 {} 尚未上传完成", asset.name));
        }
        Ok(asset.embed_markdown.trim().to_owned())
    })
}

pub fn render_local(markdown: &str, assets: &[MarkdownAsset]) -> Result<String> {
    replace_references(markdown, assets, |asset| {
        let name = format!(
            "{}-{}",
            asset.id,
            safe_display_name(&asset.name, extension(asset)?)
        );
        let alt = asset.name.replace(']', "\\]");
        Ok(format!("![{alt}](markdown-images/{name})"))
    })
}

pub fn local_file_name(asset: &MarkdownAsset) -> Result<String> {
    Ok(format!(
        "{}-{}",
        asset.id,
        safe_display_name(&asset.name, extension(asset)?)
    ))
}

fn extension(asset: &MarkdownAsset) -> Result<&'static str> {
    match asset.mime.as_str() {
        "image/png" => Ok("png"),
        "image/jpeg" => Ok("jpg"),
        _ => Err("Markdown 图片 MIME 类型不受支持".into()),
    }
}

pub fn prune_unreferenced(root: &std::path::Path, draft: &mut Value) -> Result<Vec<PathBuf>> {
    let referenced = referenced_ids(draft);
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    for mut asset in markdown_assets(draft)? {
        if referenced.contains(&asset.id) {
            if ["staged", "unreferenced"].contains(&asset.state.as_str()) {
                asset.state = "local".into();
            }
            retained.push(serde_json::to_value(asset).map_err(|e| e.to_string())?);
        } else if asset.state == "staged" {
            asset.state = "unreferenced".into();
            retained.push(serde_json::to_value(asset).map_err(|e| e.to_string())?);
        } else if let Ok(directory) = asset_directory(root, &string(draft, "id"), &asset) {
            removed.push(directory);
        }
    }
    draft["markdownAssets"] = Value::Array(retained);
    Ok(removed)
}

pub fn remove_directories(directories: &[PathBuf]) {
    for directory in directories {
        if let Err(error) = fs::remove_dir_all(directory) {
            crate::diagnostics::event("markdown_asset", &format!("orphan cleanup failed: {error}"));
        }
    }
}

pub fn asset_directory(
    root: &std::path::Path,
    draft_id: &str,
    asset: &MarkdownAsset,
) -> Result<PathBuf> {
    uuid::Uuid::parse_str(draft_id).map_err(|_| "草稿 ID 格式不正确")?;
    uuid::Uuid::parse_str(&asset.id).map_err(|_| "Markdown 图片 ID 格式不正确")?;
    let expected = root.join("markdown-assets").join(draft_id).join(&asset.id);
    let path = PathBuf::from(&asset.path);
    if path.parent() != Some(expected.as_path()) {
        return Err("Markdown 图片路径不属于当前草稿".into());
    }
    Ok(expected)
}

pub fn verify_asset_path(
    root: &std::path::Path,
    draft_id: &str,
    asset: &MarkdownAsset,
) -> Result<()> {
    let directory = asset_directory(root, draft_id, asset)?;
    let root = root
        .join("markdown-assets")
        .canonicalize()
        .map_err(|_| "Markdown 图片目录不存在")?;
    let directory = directory
        .canonicalize()
        .map_err(|_| "Markdown 图片目录不存在")?;
    let path = PathBuf::from(&asset.path)
        .canonicalize()
        .map_err(|_| "Markdown 图片不存在")?;
    if !directory.starts_with(&root)
        || path.parent() != Some(directory.as_path())
        || fs::symlink_metadata(&path)
            .map_err(|_| "Markdown 图片不存在")?
            .file_type()
            .is_symlink()
    {
        return Err("Markdown 图片路径越界".into());
    }
    Ok(())
}

pub fn draft_directory(root: &std::path::Path, draft_id: &str) -> Result<Option<PathBuf>> {
    uuid::Uuid::parse_str(draft_id).map_err(|_| "草稿 ID 格式不正确")?;
    let base = root.join("markdown-assets");
    let directory = base.join(draft_id);
    if !directory.exists() {
        return Ok(None);
    }
    let base = base
        .canonicalize()
        .map_err(|_| "Markdown 图片根目录无法读取")?;
    let directory = directory
        .canonicalize()
        .map_err(|_| "Markdown 图片目录无法读取")?;
    if directory.parent() != Some(base.as_path()) {
        return Err("Markdown 图片目录越界".into());
    }
    Ok(Some(directory))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(root: &std::path::Path, draft_id: &str, state: &str) -> MarkdownAsset {
        let id = uuid();
        let directory = root.join("markdown-assets").join(draft_id).join(&id);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("image.png");
        fs::write(&path, b"image").unwrap();
        MarkdownAsset {
            id,
            field: "actual".into(),
            name: "paste.png".into(),
            path: path.to_string_lossy().into_owned(),
            mime: "image/png".into(),
            size: 5,
            sha256: hex::encode(Sha256::digest(b"image")),
            width: 1,
            height: 1,
            state: state.into(),
            remote_id: None,
            embed_markdown: String::new(),
            embed_url: String::new(),
            error: String::new(),
        }
    }

    #[test]
    fn staged_image_survives_one_stale_save_then_is_cleaned_if_never_referenced() {
        let root = tempfile::tempdir().unwrap();
        let draft_id = uuid();
        let item = asset(root.path(), &draft_id, "staged");
        let mut draft = json!({
            "id":draft_id,
            "actual":"",
            "expected":"",
            "preconditions":"",
            "steps":"",
            "markdownAssets":[item]
        });
        assert!(prune_unreferenced(root.path(), &mut draft)
            .unwrap()
            .is_empty());
        assert_eq!(draft["markdownAssets"][0]["state"], "unreferenced");
        let removed = prune_unreferenced(root.path(), &mut draft).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(draft["markdownAssets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn referenced_staged_image_becomes_local_and_remote_render_uses_receipt() {
        let root = tempfile::tempdir().unwrap();
        let draft_id = uuid();
        let mut item = asset(root.path(), &draft_id, "staged");
        let source = format!("文字\n\n![截图](abd-asset://{})", item.id);
        let mut draft = json!({
            "id":draft_id,
            "actual":source,
            "expected":"",
            "preconditions":"",
            "steps":"",
            "markdownAssets":[item]
        });
        assert!(prune_unreferenced(root.path(), &mut draft)
            .unwrap()
            .is_empty());
        assert_eq!(draft["markdownAssets"][0]["state"], "local");
        item = markdown_assets(&draft).unwrap().remove(0);
        item.state = "uploaded".into();
        item.embed_markdown = "![云效截图](https://example.test/image.png)".into();
        let rendered = render_remote(&string(&draft, "actual"), &[item]).unwrap();
        assert!(rendered.contains("https://example.test/image.png"));
        assert!(!rendered.contains("abd-asset://"));
    }
}

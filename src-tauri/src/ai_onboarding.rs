use crate::model::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

const README: &str = include_str!("../../integrations/codex/README.md");
const MCP_EXAMPLE: &str = include_str!("../../integrations/codex/mcp-config.example.json");
const SKILL: &str = include_str!("../../integrations/codex/skills/android-bug-submission/SKILL.md");
const OPENAI_AGENT: &str =
    include_str!("../../integrations/codex/skills/android-bug-submission/agents/openai.yaml");

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiIntegrationInfo {
    pub platform: String,
    pub executable_path: String,
    pub bundle_path: String,
    pub skill_path: String,
    pub mcp_server_name: String,
    pub mcp_add_command: String,
    pub install_prompt: String,
    pub bundle_version: String,
}

fn ensure_directory(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("AI 接入资源目录不安全，请移除后重试".into());
        }
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|_| "无法创建 AI 接入资源目录".to_string())
}

fn write_bundled_file(path: &Path, contents: &str) -> Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("AI 接入资源文件不安全，请移除后重试".into());
    }
    fs::write(path, contents).map_err(|_| "无法释放 AI 接入资源".to_string())
}

fn persistent_executable_path() -> Result<PathBuf> {
    if cfg!(target_os = "linux") {
        if let Some(path) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
            if path.is_absolute() {
                return Ok(path);
            }
        }
    }
    std::env::current_exe().map_err(|_| "无法读取客户端可执行文件路径".to_string())
}

fn shell_quote(path: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("'{}'", path.replace('\'', "''"))
    } else {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }
}

fn prompt_path(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| "客户端路径无法写入 Codex 配置".to_string())?;
    if value.chars().any(char::is_control) || value.contains('`') {
        return Err("客户端路径包含 Codex 配置不支持的字符".into());
    }
    Ok(value.into())
}

fn prompt(skill_path: &str, bundle_path: &str, executable_path: &str, command: &str) -> String {
    format!(
        "请在这台电脑上为 Codex 安装 Android Bug Desk 的本机 AI 接入。客户端已把经过固定版本校验的接入资源释放到本地，请按以下步骤执行：\n\n\
1. 先阅读接入说明：`{bundle_path}/README.md`，并检查 Skill 源文件：`{skill_path}/SKILL.md`。这些文件是本次安装的唯一来源。\n\
2. 将整个 Skill 目录 `{skill_path}` 安装为 Codex Skill `android-bug-submission`。优先使用当前 `CODEX_HOME`；未设置时安装到 `~/.codex/skills/android-bug-submission`。如已有同名 Skill，先比较并用这个内置版本更新，不要创建重复目录。\n\
3. 检查 `codex mcp list`。将 MCP 服务 `android-bug-desk` 配置为 stdio，程序必须使用 `{executable_path}`，参数只能是 `--mcp-bridge`。若同名服务已有其他配置，先执行 `codex mcp remove android-bug-desk`，再执行下面的命令，不能保留重复配置：\n\
   `{command}`\n\
4. 再运行 `codex mcp list` 核对配置。若当前 Codex 会话不能立即发现新 Skill 或工具，请明确告诉我需要新开一次 Codex 会话。\n\
5. 连接后调用 `workspace_status` 验证。若客户端未运行、未登录、未选择项目或未开启“允许本机 AI 接入”，只告诉我需要在 Android Bug Desk 中完成的步骤。\n\n\
安全要求：不要读取、复制、打印或写入任何云效 PAT、OAuth token、应用数据库或附件内部路径；不要绕过 Android Bug Desk 直接调用云效提交。远端创建仍必须由使用者在客户端提交预览弹窗中确认。"
    )
}

fn materialize(root: &Path, executable_path: &Path) -> Result<AiIntegrationInfo> {
    let integration = root.join("ai-integration");
    let bundle = integration.join("codex");
    let skills = bundle.join("skills");
    let skill = skills.join("android-bug-submission");
    let agents = skill.join("agents");
    for directory in [&integration, &bundle, &skills, &skill, &agents] {
        ensure_directory(directory)?;
    }
    write_bundled_file(&bundle.join("README.md"), README)?;
    write_bundled_file(&bundle.join("mcp-config.example.json"), MCP_EXAMPLE)?;
    write_bundled_file(&skill.join("SKILL.md"), SKILL)?;
    write_bundled_file(&agents.join("openai.yaml"), OPENAI_AGENT)?;

    let executable = prompt_path(executable_path)?;
    let bundle_text = prompt_path(&bundle)?;
    let skill_text = prompt_path(&skill)?;
    let command = format!(
        "codex mcp add android-bug-desk -- {} --mcp-bridge",
        shell_quote(&executable)
    );
    let digest = hex::encode(Sha256::digest(
        [README, MCP_EXAMPLE, SKILL, OPENAI_AGENT]
            .concat()
            .as_bytes(),
    ));
    let manifest = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "sha256": digest,
        "skill": "android-bug-submission",
        "mcpServer": "android-bug-desk"
    });
    write_bundled_file(
        &bundle.join("manifest.json"),
        &serde_json::to_string_pretty(&manifest).map_err(|_| "无法生成 AI 接入清单")?,
    )?;

    Ok(AiIntegrationInfo {
        platform: std::env::consts::OS.into(),
        executable_path: executable.clone(),
        bundle_path: bundle_text.clone(),
        skill_path: skill_text.clone(),
        mcp_server_name: "android-bug-desk".into(),
        mcp_add_command: command.clone(),
        install_prompt: prompt(&skill_text, &bundle_text, &executable, &command),
        bundle_version: env!("CARGO_PKG_VERSION").into(),
    })
}

#[tauri::command]
pub fn prepare_ai_integration(app: tauri::AppHandle) -> Result<AiIntegrationInfo> {
    let root = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "无法读取应用数据目录".to_string())?;
    materialize(&root, &persistent_executable_path()?)
}

#[tauri::command]
pub fn copy_ai_install_prompt(app: tauri::AppHandle) -> Result<AiIntegrationInfo> {
    let info = prepare_ai_integration(app)?;
    arboard::Clipboard::new()
        .map_err(|_| "剪贴板不可用")?
        .set_text(info.install_prompt.clone())
        .map_err(|_| "复制失败")?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_bundled_skill_and_generates_scoped_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp
            .path()
            .join("Android Bug Desk")
            .join("android-bug-desk");
        let info = materialize(temp.path(), &executable).unwrap();

        assert_eq!(
            fs::read_to_string(Path::new(&info.skill_path).join("SKILL.md")).unwrap(),
            SKILL
        );
        assert!(Path::new(&info.bundle_path).join("manifest.json").is_file());
        assert!(info.install_prompt.contains(&info.skill_path));
        assert!(info.install_prompt.contains(&info.executable_path));
        assert!(info.install_prompt.contains("workspace_status"));
        assert!(info.install_prompt.contains("不要读取、复制、打印"));
        assert!(!info
            .install_prompt
            .to_lowercase()
            .contains("authorization:"));
        assert!(info.mcp_add_command.contains("--mcp-bridge"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_resource_directory() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), temp.path().join("ai-integration")).unwrap();
        let error = materialize(temp.path(), Path::new("/tmp/android-bug-desk")).unwrap_err();
        assert!(error.contains("不安全"));
    }
}

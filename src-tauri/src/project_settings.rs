use crate::{commands::AppState, model::Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use tauri::State;

const MAX_ENTRIES: usize = 500;
const MAX_ID_LEN: usize = 128;
const MAX_PACKAGE_LEN: usize = 255;
const SETTINGS_PREFIX: &str = "project-settings:";

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSetting {
    pub project_id: String,
    pub enabled: bool,
    pub package_name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSettings {
    pub entries: Vec<ProjectSetting>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftProjectScope {
    pub organization_id: String,
    pub project_id: String,
}

fn valid_id(value: &str) -> bool {
    value.len() <= MAX_ID_LEN
        && !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_package(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_PACKAGE_LEN {
        return value.is_empty();
    }
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphabetic()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

fn validate_organization(organization_id: &str) -> Result<()> {
    if valid_id(organization_id) {
        Ok(())
    } else {
        Err("组织 ID 格式不正确".into())
    }
}

fn normalize(mut settings: ProjectSettings) -> Result<ProjectSettings> {
    if settings.entries.len() > MAX_ENTRIES {
        return Err(format!("项目映射最多支持 {} 条", MAX_ENTRIES));
    }
    let mut ids = HashSet::with_capacity(settings.entries.len());
    for entry in &mut settings.entries {
        entry.project_id = entry.project_id.trim().to_string();
        entry.package_name = entry.package_name.trim().to_string();
        if !valid_id(&entry.project_id) {
            return Err("项目 ID 格式不正确".into());
        }
        if !ids.insert(entry.project_id.clone()) {
            return Err("项目映射中存在重复的项目 ID".into());
        }
        if !valid_package(&entry.package_name) {
            return Err(format!("项目 {} 的包名格式不正确", entry.project_id));
        }
    }
    Ok(settings)
}

fn settings_key(user_id: &str, organization_id: &str) -> String {
    // Both components come from verified identity or validated input. Keeping the
    // scope in the key prevents settings from leaking across users or organizations.
    format!("{SETTINGS_PREFIX}{user_id}:{organization_id}")
}

/// Returns only the current user's valid project scope for each locally stored draft.
/// The persisted scope itself is deliberately kept out of the renderer response.
pub fn draft_scopes_for_state(
    state: &AppState,
    drafts: &[Value],
) -> Result<BTreeMap<String, DraftProjectScope>> {
    let user = state.user.lock().unwrap();
    let Some(user_id) = user
        .as_ref()
        .map(|identity| identity.id.as_str())
        .filter(|id| valid_id(id))
    else {
        return Ok(BTreeMap::new());
    };
    let mut result = BTreeMap::new();
    for draft in drafts {
        let draft_id = crate::model::string(draft, "id");
        if draft_id.is_empty() {
            continue;
        }
        let Some(scope) = state.store.get::<String>("draft_scope", &draft_id)? else {
            continue;
        };
        let parts: Vec<&str> = scope.split(':').collect();
        if parts.len() != 4
            || parts[0] != user_id
            || !valid_id(parts[1])
            || !valid_id(parts[2])
            || (!parts[3].is_empty() && !valid_id(parts[3]))
        {
            continue;
        }
        result.insert(
            draft_id,
            DraftProjectScope {
                organization_id: parts[1].to_string(),
                project_id: parts[2].to_string(),
            },
        );
    }
    Ok(result)
}

pub fn load_for_state(state: &AppState, organization_id: &str) -> Result<ProjectSettings> {
    validate_organization(organization_id)?;
    // Keep the verified identity guard until the scoped store operation completes.
    // This prevents account switching between authorization and the read/write.
    let user = state.user.lock().unwrap();
    let user_id = user
        .as_ref()
        .map(|identity| identity.id.as_str())
        .filter(|id| valid_id(id))
        .ok_or_else(|| "请先验证登录令牌".to_string())?;
    state
        .store
        .get("settings", &settings_key(user_id, organization_id))
        .and_then(|value| match value {
            Some(saved) => normalize(saved).map_err(|error| format!("项目映射设置已损坏：{error}")),
            None => Ok(ProjectSettings::default()),
        })
}

/// The saved project mapping is the only trusted package source for commands.
pub fn current_package_for_state(state: &AppState) -> Result<String> {
    let profile = state.profile.lock().unwrap().clone();
    if profile.organization_id == "unconfigured" || profile.project_id == "unconfigured" {
        return Err("请选择组织与项目".into());
    }
    let settings = load_for_state(state, &profile.organization_id)?;
    let entry = settings
        .entries
        .into_iter()
        .find(|entry| entry.project_id == profile.project_id && entry.enabled)
        .ok_or("当前项目未在设置中启用，请先完成项目配置")?;
    if entry.package_name.is_empty() {
        return Err("当前项目未配置 Android 包名，请前往设置补全后再操作".into());
    }
    crate::adb::valid_package(&entry.package_name)?;
    Ok(entry.package_name)
}

pub fn apply_current_package(state: &AppState, draft: &mut Value) -> Result<String> {
    let package = current_package_for_state(state)?;
    let detail = "来自设置页当前项目包名映射";
    if crate::model::string(draft, "app.packageName") != package
        || draft["sources"]["app.packageName"]["detail"]
            .as_str()
            .unwrap_or("")
            != detail
    {
        draft["app"]["packageName"] = serde_json::json!(package);
        draft["sources"]["app.packageName"] = serde_json::json!({
            "status":"manual",
            "at":crate::model::now(),
            "detail":detail
        });
    }
    Ok(package)
}

pub fn save_for_state(
    state: &AppState,
    organization_id: &str,
    settings: ProjectSettings,
) -> Result<ProjectSettings> {
    validate_organization(organization_id)?;
    let settings = normalize(settings)?;
    // Keep the verified identity guard until the scoped store operation completes.
    let user = state.user.lock().unwrap();
    let user_id = user
        .as_ref()
        .map(|identity| identity.id.as_str())
        .filter(|id| valid_id(id))
        .ok_or_else(|| "请先验证登录令牌".to_string())?;
    state.store.put(
        "settings",
        &settings_key(user_id, organization_id),
        &settings,
    )?;
    Ok(settings)
}

#[tauri::command]
pub fn load_project_settings(
    organization_id: String,
    state: State<'_, AppState>,
) -> Result<ProjectSettings> {
    load_for_state(&state, &organization_id)
}

#[tauri::command]
pub fn save_project_settings(
    organization_id: String,
    settings: ProjectSettings,
    state: State<'_, AppState>,
) -> Result<ProjectSettings> {
    save_for_state(&state, &organization_id, settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::AppState, workspace::UserIdentity};
    use serde_json::json;
    use tempfile::tempdir;

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempdir().unwrap();
        let state = AppState::new(dir.path().to_path_buf()).unwrap();
        (dir, state)
    }

    fn authenticate(state: &AppState, user_id: &str) {
        *state.user.lock().unwrap() = Some(UserIdentity {
            id: user_id.into(),
            name: "测试用户".into(),
            ..Default::default()
        });
    }

    fn sample(project_id: &str, package_name: &str) -> ProjectSettings {
        ProjectSettings {
            entries: vec![ProjectSetting {
                project_id: project_id.into(),
                enabled: true,
                package_name: package_name.into(),
            }],
        }
    }

    #[test]
    fn persists_across_state_restart_and_trims_values() {
        let (dir, first) = state();
        authenticate(&first, "user-a");
        let mut input = sample(" project-a ", " ");
        input.entries[0].enabled = false;
        let saved = save_for_state(&first, "org-a", input).unwrap();
        assert_eq!(saved.entries[0].project_id, "project-a");
        assert!(!saved.entries[0].enabled);
        assert!(saved.entries[0].package_name.is_empty());
        drop(first);
        let reopened = AppState::new(dir.path().to_path_buf()).unwrap();
        authenticate(&reopened, "user-a");
        assert_eq!(load_for_state(&reopened, "org-a").unwrap(), saved);
    }

    #[test]
    fn separates_users_and_organizations() {
        let (_dir, state) = state();
        authenticate(&state, "alice");
        save_for_state(&state, "org-a", sample("project-a", "com.a")).unwrap();
        save_for_state(&state, "org-b", sample("project-b", "com.b")).unwrap();
        assert_eq!(
            load_for_state(&state, "org-a").unwrap().entries[0].project_id,
            "project-a"
        );
        assert_eq!(
            load_for_state(&state, "org-b").unwrap().entries[0].project_id,
            "project-b"
        );
        state.user.lock().unwrap().as_mut().unwrap().id = "bob".into();
        assert!(load_for_state(&state, "org-a").unwrap().entries.is_empty());
    }

    #[test]
    fn requires_authentication_and_rejects_bad_input() {
        let (_dir, state) = state();
        assert_eq!(
            load_for_state(&state, "org-a").unwrap_err(),
            "请先验证登录令牌"
        );
        assert!(save_for_state(&state, "org-a", sample("project-a", "com.a")).is_err());
        authenticate(&state, "alice");
        assert!(save_for_state(&state, "bad org", sample("project-a", "com.a")).is_err());
        assert!(save_for_state(&state, "org-a", sample("bad project", "com.a")).is_err());
        assert!(save_for_state(
            &state,
            &"o".repeat(MAX_ID_LEN + 1),
            sample("project-a", "com.a")
        )
        .is_err());
        assert!(save_for_state(&state, "org-a", sample("project-a", "not valid")).is_err());
        assert!(save_for_state(&state, "org-a", sample("project-a", "android")).is_err());
        assert!(save_for_state(
            &state,
            "org-a",
            sample("project-a", &format!("com.{}", "a".repeat(MAX_PACKAGE_LEN))),
        )
        .is_err());
        assert!(save_for_state(
            &state,
            "org-a",
            ProjectSettings {
                entries: vec![
                    ProjectSetting {
                        project_id: "same".into(),
                        enabled: true,
                        package_name: "com.a".into()
                    },
                    ProjectSetting {
                        project_id: "same".into(),
                        enabled: false,
                        package_name: "com.b".into()
                    },
                ],
            },
        )
        .is_err());

        // A rejected update must leave the previously persisted value intact.
        let saved = save_for_state(&state, "org-a", sample("project-a", "com.good")).unwrap();
        assert!(save_for_state(&state, "org-a", sample("project-a", "bad value")).is_err());
        assert_eq!(load_for_state(&state, "org-a").unwrap(), saved);
    }

    #[test]
    fn rejects_corrupted_persisted_record_instead_of_hiding_it() {
        let (_dir, state) = state();
        authenticate(&state, "alice");
        let key = settings_key("alice", "org-a");
        state
            .store
            .put(
                "settings",
                &key,
                &ProjectSettings {
                    entries: vec![ProjectSetting {
                        project_id: "project-a".into(),
                        enabled: true,
                        package_name: "not-a-package".into(),
                    }],
                },
            )
            .unwrap();
        let error = load_for_state(&state, "org-a").unwrap_err();
        assert!(error.contains("项目映射设置已损坏"));
    }

    #[test]
    fn rejects_excessive_entries() {
        let (_dir, state) = state();
        authenticate(&state, "alice");
        let entries = (0..=MAX_ENTRIES)
            .map(|i| ProjectSetting {
                project_id: format!("p{i}"),
                enabled: true,
                package_name: String::new(),
            })
            .collect();
        assert!(save_for_state(&state, "org-a", ProjectSettings { entries }).is_err());
    }

    #[test]
    fn current_project_package_is_required_authoritative_and_stable() {
        let (_dir, state) = state();
        authenticate(&state, "alice");
        state.profile.lock().unwrap().organization_id = "org-a".into();
        state.profile.lock().unwrap().project_id = "project-a".into();
        assert!(current_package_for_state(&state)
            .unwrap_err()
            .contains("未在设置中启用"));
        save_for_state(&state, "org-a", sample("project-a", "")).unwrap();
        assert!(current_package_for_state(&state)
            .unwrap_err()
            .contains("未配置 Android 包名"));
        save_for_state(&state, "org-a", sample("project-a", "com.configured.app")).unwrap();
        assert_eq!(
            current_package_for_state(&state).unwrap(),
            "com.configured.app"
        );
        let mut draft: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
        apply_current_package(&state, &mut draft).unwrap();
        assert_eq!(draft["app"]["packageName"], "com.configured.app");
        let source = draft["sources"]["app.packageName"].clone();
        apply_current_package(&state, &mut draft).unwrap();
        assert_eq!(draft["sources"]["app.packageName"], source);
    }

    #[test]
    fn draft_scope_attribution_filters_other_users_and_malformed_records() {
        let (_dir, state) = state();
        authenticate(&state, "alice");
        let drafts = vec![
            json!({"id":"draft-current"}),
            json!({"id":"draft-old"}),
            json!({"id":"draft-other"}),
            json!({"id":"draft-bad"}),
            json!({"id":"draft-missing"}),
        ];
        state
            .store
            .put(
                "draft_scope",
                "draft-current",
                &"alice:org-current:project-current:",
            )
            .unwrap();
        state
            .store
            .put(
                "draft_scope",
                "draft-old",
                &"alice:org-old:project-old:type-1",
            )
            .unwrap();
        state
            .store
            .put("draft_scope", "draft-other", &"bob:org-b:project-b:")
            .unwrap();
        state
            .store
            .put("draft_scope", "draft-bad", &"alice:org bad:project-b:")
            .unwrap();
        let scopes = draft_scopes_for_state(&state, &drafts).unwrap();
        assert_eq!(scopes["draft-current"].project_id, "project-current");
        assert_eq!(scopes["draft-old"].organization_id, "org-old");
        assert!(!scopes.contains_key("draft-other"));
        assert!(!scopes.contains_key("draft-bad"));
        assert!(!scopes.contains_key("draft-missing"));
        state.user.lock().unwrap().take();
        assert!(draft_scopes_for_state(&state, &drafts).unwrap().is_empty());
    }
}

use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
    pub writes: std::sync::Arc<std::sync::Mutex<()>>,
}
impl Store {
    pub fn new(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let store = Self {
            root: root.into(),
            writes: Default::default(),
        };
        store.connect()?.execute_batch("CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id));").map_err(|e| e.to_string())?;
        store.migrate_removed_fields()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(store)
    }
    fn migrate_removed_fields(&self) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        let rows = {
            let mut stmt = tx
                .prepare("SELECT kind,id,data FROM objects")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for (kind, id, raw) in rows {
            let mut value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            let mut changed = false;
            let remove = |object: &mut serde_json::Map<String, Value>, key: &str| {
                object.remove(key).is_some()
            };
            if kind == "draft" {
                if let Some(object) = value.as_object_mut() {
                    for key in [
                        "business",
                        "buildUrl",
                        "buildStatus",
                        "buildManifest",
                        "manualNote",
                        "cloud",
                    ] {
                        changed |= remove(object, key);
                    }
                    if !object.contains_key("jenkinsPackageUrl") {
                        object.insert("jenkinsPackageUrl".into(), Value::String(String::new()));
                        changed = true;
                    }
                    if let Some(app) = object.get_mut("app").and_then(Value::as_object_mut) {
                        for key in ["channel", "environment", "obfuscation", "hardening"] {
                            changed |= remove(app, key);
                        }
                    }
                    if let Some(event) = object.get_mut("event").and_then(Value::as_object_mut) {
                        changed |= remove(event, "lateCapture");
                    }
                    if let Some(sources) = object.get_mut("sources").and_then(Value::as_object_mut)
                    {
                        let old = sources.len();
                        sources.retain(|key, _| {
                            !matches!(
                                key.as_str(),
                                "business"
                                    | "buildUrl"
                                    | "buildStatus"
                                    | "buildManifest"
                                    | "manualNote"
                                    | "app.channel"
                                    | "app.environment"
                                    | "app.obfuscation"
                                    | "app.hardening"
                                    | "event.lateCapture"
                            ) && !key.starts_with("cloud.")
                        });
                        changed |= sources.len() != old;
                    }
                }
            } else if kind == "session" {
                if let Some(object) = value.as_object_mut() {
                    changed |= remove(object, "business");
                    changed |= remove(object, "buildUrl");
                }
            } else if kind == "selection" {
                if let Some(object) = value.as_object_mut() {
                    changed |= remove(object, "business");
                }
            } else if kind == "settings" && id.starts_with("profile:") {
                if let Some(object) = value.as_object_mut() {
                    changed |= remove(object, "businessProfiles");
                }
            } else if matches!(kind.as_str(), "submission" | "metadata") {
                if let Some(profile) = value.get_mut("profile").and_then(Value::as_object_mut) {
                    changed |= remove(profile, "businessProfiles");
                }
            }
            if changed {
                tx.execute(
                    "UPDATE objects SET data=?3 WHERE kind=?1 AND id=?2",
                    params![
                        kind,
                        id,
                        serde_json::to_string(&value).map_err(|e| e.to_string())?
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.execute_batch("PRAGMA user_version=2;")
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn connect(&self) -> Result<Connection> {
        let c = Connection::open(self.root.join("state.sqlite")).map_err(|e| e.to_string())?;
        c.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(|e| e.to_string())?;
        Ok(c)
    }
    pub fn put<T: serde::Serialize>(&self, kind: &str, id: &str, data: &T) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO objects(kind,id,data) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data", params![kind,id,serde_json::to_string(data).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn put_many(&self, objects: &[(&str, &str, &Value)]) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        for (kind, id, value) in objects {
            tx.execute("INSERT INTO objects(kind,id,data) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data", params![kind,id,serde_json::to_string(value).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn get<T: serde::de::DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM objects WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|r| serde_json::from_str(&r).map_err(|e| e.to_string()))
            .transpose()
    }
    pub(crate) fn remove_many(&self, objects: &[(&str, &str)]) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        for (kind, id) in objects {
            tx.execute(
                "DELETE FROM objects WHERE kind=?1 AND id=?2",
                params![kind, id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn list(&self, kind: &str) -> Result<Vec<Value>> {
        let c = self.connect()?;
        let mut stmt = c
            .prepare("SELECT data FROM objects WHERE kind=?1 ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([kind], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn draft(&self, id: &str) -> Result<Value> {
        self.get("draft", id)?.ok_or_else(|| "草稿不存在".into())
    }
    pub fn editable(&self, id: &str) -> Result<()> {
        if self.get::<Submission>("submission", id)?.is_some() {
            return Err("证据已固定，请创建新草稿；补传从提交记录恢复".into());
        }
        Ok(())
    }
    pub fn recover(&self) -> Result<()> {
        for value in self.list("submission")? {
            let mut s: Submission = serde_json::from_value(value).map_err(|e| e.to_string())?;
            if s.state == "creating" {
                s.state = "create_unknown".into();
                s.error = Some("创建时客户端退出，需先核对远端记录".into());
            } else if ["uploading", "finalizing", "created"].contains(&s.state.as_str()) {
                s.state = "partial".into();
            }
            for a in &mut s.uploads {
                if a.state == "uploading" {
                    a.state = "unknown".into();
                }
            }
            self.put("submission", &s.draft_id, &s)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removed_fields_are_migrated_before_strict_deserialization() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        let legacy_profile = json!({
            "name":"旧项目","edition":"central","organizationId":"org-a",
            "projectId":"project-a","businessProfiles":["Android 测试"],
            "requiredFieldIds":[],"fieldMappings":{},"enableVersions":true,
            "enableSprints":true,"attachmentWarningMb":50.0
        });
        store
            .put("settings", "profile:user-a", &legacy_profile)
            .unwrap();
        store
            .put(
                "submission",
                "draft-a",
                &json!({
                    "draftId":"draft-a","profile":legacy_profile,"state":"submitted",
                    "evidence":{"id":"evidence-a","directory":"/tmp/evidence-a",
                        "zipPath":"/tmp/evidence-a.zip","sha256":"a".repeat(64),"files":[]},
                    "workitemId":"1","identifier":"BUG-1","url":null,"uploads":[],
                    "error":null,"intendedAt":"2026-10-12T00:00:00Z",
                    "updatedAt":"2026-10-12T00:00:00Z"
                }),
            )
            .unwrap();
        store
            .put(
                "draft",
                "draft-a",
                &json!({
                    "business":"旧方向","buildUrl":"旧链接","buildStatus":"verified",
                    "buildManifest":{},"manualNote":"旧备注","cloud":{},
                    "app":{"packageName":"com.example","versionName":"1","versionCode":"1",
                        "channel":"旧渠道","environment":"旧环境","obfuscation":"旧混淆","hardening":"旧加固"},
                    "event":{"occurredAt":"","originalAt":"","source":"manual","sourceNote":"",
                        "lateCapture":true},
                    "sources":{"business":{},"cloud.value":{},"app.channel":{}}
                }),
            )
            .unwrap();
        store
            .put(
                "session",
                "session-a",
                &json!({"business":"旧方向","buildUrl":"旧链接","id":"session-a"}),
            )
            .unwrap();
        store
            .put(
                "selection",
                "selection-a",
                &json!({"business":"旧方向","kind":"UI"}),
            )
            .unwrap();
        drop(store);

        let migrated = Store::new(root.path()).unwrap();
        let profile: Profile = migrated.get("settings", "profile:user-a").unwrap().unwrap();
        assert_eq!(profile.project_id, "project-a");
        let submission: Submission = migrated.get("submission", "draft-a").unwrap().unwrap();
        assert_eq!(submission.profile.project_id, "project-a");
        let draft: Value = migrated.get("draft", "draft-a").unwrap().unwrap();
        assert_eq!(draft["jenkinsPackageUrl"], "");
        for key in [
            "business",
            "buildUrl",
            "buildStatus",
            "buildManifest",
            "manualNote",
            "cloud",
        ] {
            assert!(draft.get(key).is_none(), "{key}");
        }
        for key in ["channel", "environment", "obfuscation", "hardening"] {
            assert!(draft["app"].get(key).is_none(), "{key}");
        }
        assert!(draft["event"].get("lateCapture").is_none());
        assert!(draft["sources"].as_object().unwrap().is_empty());
        let session: Value = migrated.get("session", "session-a").unwrap().unwrap();
        assert!(session.get("business").is_none());
        assert!(session.get("buildUrl").is_none());
        let selection: Value = migrated.get("selection", "selection-a").unwrap().unwrap();
        assert!(selection.get("business").is_none());
        migrated.recover().unwrap();
    }
}

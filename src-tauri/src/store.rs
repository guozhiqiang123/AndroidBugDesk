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
        store.connect()?.execute_batch("CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id)); PRAGMA user_version=1;").map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(store)
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

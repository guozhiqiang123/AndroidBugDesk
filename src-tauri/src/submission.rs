use crate::{
    evidence,
    model::*,
    store::Store,
    validation,
    yunxiao::{create_payload, Api},
};
use serde_json::json;
use std::path::Path;
pub struct Submitter {
    pub store: Store,
}
impl Submitter {
    fn save(&self, s: &mut Submission) -> Result<()> {
        s.updated_at = now();
        self.store.put("submission", &s.draft_id, s)
    }
    pub async fn submit(&self, id: &str, api: &Api) -> Result<Submission> {
        let d = self.store.draft(id)?;
        if let Some(mut old) = self.store.get::<Submission>("submission", id)? {
            if old.profile.project_id != api.profile.project_id
                || old.profile.organization_id != api.profile.organization_id
            {
                return Err("请恢复原提交项目，不能向其他项目补传".into());
            }
            if old.state == "submitted" || old.state == "create_unknown" {
                return Ok(old);
            }
            return self.finish(api, &mut old).await;
        }
        let metadata = api.metadata(&string(&d, "workitemTypeId")).await?;
        let errors = validation::validate(&d, &api.profile, Some(&metadata), true, true);
        if !errors.is_empty() {
            return Err(serde_json::to_string(&errors).unwrap());
        }
        for a in &attachments(&d)? {
            evidence::verify_attachment(a)?;
        }
        let markdown_assets = crate::markdown_assets::snapshot(&self.store.root, &d)?;
        let evidence = evidence::build(&self.store, &d, &api.profile, false)?;
        let mut files = evidence.files.clone();
        let (size, _) = evidence::hash_file(Path::new(&evidence.zip_path))?;
        files.push(Attachment {
            id: format!("zip-{}", evidence.id),
            name: format!("evidence-{}.zip", evidence.id),
            path: evidence.zip_path.clone(),
            mime: "application/zip".into(),
            size,
            sha256: evidence.sha256.clone(),
            role: "report".into(),
            source: "客户端生成".into(),
            coverage_start: String::new(),
            coverage_end: String::new(),
            confirmed: true,
            derived_from: None,
        });
        let uploads = files
            .iter()
            .map(|a| Upload {
                local_id: a.id.clone(),
                name: format!(
                    "{}-{}-{}",
                    evidence.id,
                    &a.sha256[..12],
                    evidence::safe_name(&a.name)
                ),
                sha256: a.sha256.clone(),
                remote_id: None,
                state: "pending".into(),
            })
            .collect();
        let mut s = Submission {
            draft_id: id.into(),
            profile: api.profile.clone(),
            state: "ready".into(),
            evidence,
            workitem_id: None,
            identifier: None,
            url: None,
            uploads,
            markdown_assets,
            error: None,
            intended_at: now(),
            updated_at: now(),
        };
        let description = format!(
            "{}\n\n{}",
            evidence::report(&d, &api.profile),
            evidence::managed(&s.evidence, &s.uploads, false)
        );
        let payload = create_payload(&d, &metadata, description)?;
        s.state = "creating".into();
        self.save(&mut s)?;
        match api.create(payload).await {
            Ok(id) => {
                s.workitem_id = Some(id);
                s.state = "created".into();
                self.save(&mut s)?;
                self.finish(api, &mut s).await
            }
            Err(error) => {
                s.state = "create_unknown".into();
                s.error = Some(error);
                self.save(&mut s)?;
                Ok(s)
            }
        }
    }
    fn upload_files(s: &Submission) -> Result<Vec<Attachment>> {
        let mut files = s.evidence.files.clone();
        let (size, sha256) = evidence::hash_file(Path::new(&s.evidence.zip_path))?;
        if sha256 != s.evidence.sha256 {
            return Err("不可变证据包已被修改，停止补传".into());
        }
        files.push(Attachment {
            id: format!("zip-{}", s.evidence.id),
            name: "evidence.zip".into(),
            path: s.evidence.zip_path.clone(),
            mime: "application/zip".into(),
            size,
            sha256,
            role: "report".into(),
            source: "客户端生成".into(),
            coverage_start: String::new(),
            coverage_end: String::new(),
            confirmed: true,
            derived_from: None,
        });
        Ok(files)
    }
    fn markdown_attachment(asset: &MarkdownAsset) -> Attachment {
        Attachment {
            id: asset.id.clone(),
            name: asset.name.clone(),
            path: asset.path.clone(),
            mime: asset.mime.clone(),
            size: asset.size,
            sha256: asset.sha256.clone(),
            role: "image".into(),
            source: "Markdown 编辑器粘贴".into(),
            coverage_start: String::new(),
            coverage_end: String::new(),
            confirmed: true,
            derived_from: None,
        }
    }
    async fn upload_markdown_assets(
        &self,
        api: &Api,
        workitem_id: &str,
        submission: &mut Submission,
    ) -> Result<()> {
        for index in 0..submission.markdown_assets.len() {
            let current = submission.markdown_assets[index].clone();
            if current.state == "uploaded" {
                continue;
            }
            if ["uploading", "unknown"].contains(&current.state.as_str()) {
                return Err(format!(
                    "正文图片 {} 上次上传结果不确定；为避免重复写入，请先在云效核对",
                    current.name
                ));
            }
            crate::markdown_assets::verify_integrity(
                &self.store.root,
                &submission.draft_id,
                &current,
            )?;
            let attachment = Self::markdown_attachment(&current);
            let remote_name = format!(
                "inline-{}-{}",
                current.id,
                evidence::safe_name(&current.name)
            );
            submission.state = "uploading".into();
            submission.markdown_assets[index].state = "uploading".into();
            submission.markdown_assets[index].error.clear();
            self.save(submission)?;
            match api.upload(workitem_id, &attachment, &remote_name).await {
                Ok(remote) => {
                    let embed_markdown = if !remote.embed_markdown.trim().is_empty() {
                        remote.embed_markdown.trim().to_owned()
                    } else if !remote.embed_url.trim().is_empty() {
                        let alt = current.name.replace(']', "\\]");
                        format!("![{alt}]({})", remote.embed_url.trim())
                    } else {
                        submission.markdown_assets[index].state = "unknown".into();
                        submission.markdown_assets[index].remote_id = Some(remote.id);
                        submission.markdown_assets[index].error =
                            "云效附件响应缺少正文嵌入地址".into();
                        self.save(submission)?;
                        return Err("云效附件响应缺少正文嵌入地址，停止覆盖描述".into());
                    };
                    submission.markdown_assets[index].remote_id = Some(remote.id);
                    submission.markdown_assets[index].embed_markdown = embed_markdown;
                    submission.markdown_assets[index].embed_url = remote.embed_url;
                    submission.markdown_assets[index].state = "uploaded".into();
                    submission.markdown_assets[index].error.clear();
                    self.save(submission)?;
                }
                Err(error) => {
                    submission.markdown_assets[index].state = if error.starts_with("[未写入]") {
                        "local"
                    } else {
                        "unknown"
                    }
                    .into();
                    submission.markdown_assets[index].error = error.clone();
                    self.save(submission)?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    async fn finish(&self, api: &Api, s: &mut Submission) -> Result<Submission> {
        let result = self.finish_inner(api, s).await;
        if let Err(e) = result {
            s.state = "partial".into();
            s.error = Some(e);
            self.save(s)?;
        }
        Ok(s.clone())
    }
    async fn finish_inner(&self, api: &Api, s: &mut Submission) -> Result<()> {
        let id = s
            .workitem_id
            .clone()
            .ok_or("缺少真实工作项 ID，不能补传或重新创建")?;
        self.upload_markdown_assets(api, &id, s).await?;
        let files = Self::upload_files(s)?;
        for (i, a) in files.iter().enumerate() {
            evidence::verify_attachment(a)?;
            if s.uploads[i].state == "confirmed" {
                continue;
            }
            let mut rows = api.attachments(&id).await?;
            if ["unknown", "uploading"].contains(&s.uploads[i].state.as_str()) {
                let candidates = rows
                    .iter()
                    .filter(|r| string(r, "fileName") == s.uploads[i].name)
                    .collect::<Vec<_>>();
                let mut matched = None;
                for row in candidates {
                    if api.matches_download(row, a).await? {
                        matched = Some(string(row, "id"));
                        break;
                    }
                }
                if let Some(remote) = matched {
                    s.uploads[i].remote_id = Some(remote);
                    s.uploads[i].state = "confirmed".into();
                    self.save(s)?;
                    continue;
                }
                return Err(format!(
                    "附件 {} 上次上传结果不确定，未核对清楚前不会重复上传",
                    a.name
                ));
            }
            s.state = "uploading".into();
            s.uploads[i].state = "uploading".into();
            self.save(s)?;
            match api.upload(&id, a, &s.uploads[i].name).await {
                Ok(remote) => {
                    s.uploads[i].remote_id = Some(remote.id);
                    s.uploads[i].state = "unknown".into();
                    self.save(s)?;
                }
                Err(e) => {
                    s.uploads[i].state = if e.starts_with("[未写入]") {
                        "pending"
                    } else {
                        "unknown"
                    }
                    .into();
                    self.save(s)?;
                    return Err(e);
                }
            }
            rows = api.attachments(&id).await?;
            let row = rows
                .iter()
                .find(|r| {
                    string(r, "id") == s.uploads[i].remote_id.clone().unwrap_or_default()
                        || string(r, "fileId") == s.uploads[i].remote_id.clone().unwrap_or_default()
                })
                .ok_or("附件上传后列表未找到，保持核对状态")?;
            if !api.matches_download(row, a).await? {
                return Err("附件下载 SHA256 不匹配，停止提交完成".into());
            }
            s.uploads[i].remote_id = Some(string(row, "id"));
            s.uploads[i].state = "confirmed".into();
            self.save(s)?;
        }
        s.state = "finalizing".into();
        self.save(s)?;
        let remote = api.get(&id).await?;
        if string(&remote, "space.id") != s.profile.project_id {
            return Err("工作项不属于当前项目，停止更新".into());
        }
        let description = evidence::merge_managed(
            &string(&remote, "description"),
            &s.evidence,
            &s.uploads,
            true,
        )?;
        let description = crate::markdown_assets::render_remote(&description, &s.markdown_assets)?;
        // A second read detects edits while attachments were being reconciled. API offers no cross-request CAS.
        let latest = api.get(&id).await?;
        if latest["description"] != remote["description"] {
            return Err("云效描述被他人修改，请重新核对后恢复".into());
        }
        api.update_description(&id, &description).await?;
        let confirmed = api.get(&id).await?;
        if string(&confirmed, "description") != description {
            return Err("最终描述回读不一致，保留部分提交状态".into());
        }
        let rows = api.attachments(&id).await?;
        if s.uploads.iter().any(|a| {
            !rows.iter().any(|r| {
                Some(string(r, "id")) == a.remote_id || Some(string(r, "fileId")) == a.remote_id
            })
        }) {
            return Err("最终附件回读不完整".into());
        }
        s.identifier = Some(string(&confirmed, "serialNumber"));
        let url = string(&confirmed, "url");
        s.url = crate::workitem_links::bug_url(Some(&url), s.identifier.as_deref());
        s.state = "submitted".into();
        s.error = None;
        self.save(s)?;
        std::fs::write(Path::new(&s.evidence.directory).with_extension("receipt.json"),serde_json::to_vec_pretty(&json!({"organizationId":s.profile.organization_id,"projectId":s.profile.project_id,"workitemId":s.workitem_id,"identifier":s.identifier,"url":s.url,"attachments":s.uploads,"markdownAssets":s.markdown_assets,"evidenceSha256":s.evidence.sha256,"submittedAt":now(),"state":s.state})).unwrap()).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub async fn reconcile(
        &self,
        id: &str,
        api: &Api,
        remote_id: Option<String>,
    ) -> Result<Submission> {
        let mut s: Submission = self
            .store
            .get("submission", id)?
            .ok_or("没有需要核对的提交")?;
        if s.profile.project_id != api.profile.project_id
            || s.profile.organization_id != api.profile.organization_id
        {
            return Err("请切换回原提交的组织和项目后再核对".into());
        }
        if s.state != "create_unknown" {
            return Ok(s);
        }
        let candidates = if let Some(id) = remote_id {
            vec![json!({"id":id})]
        } else {
            api.candidates(&string(&self.store.draft(id)?, "title"))
                .await?
        };
        let mut matches = Vec::new();
        for candidate in candidates {
            let rid = string(&candidate, "id");
            let row = api.get(&rid).await?;
            if string(&row, "space.id") == s.profile.project_id
                && string(&row, "description").contains(&evidence::marker(&s.evidence.id))
            {
                matches.push(rid);
            }
        }
        if matches.len() != 1 {
            s.error =
                Some("未唯一确认已创建缺陷；请在云效检查并提供工作项 ID，不会重新创建".into());
            self.save(&mut s)?;
            return Ok(s);
        }
        s.workitem_id = matches.pop();
        s.state = "created".into();
        self.save(&mut s)?;
        self.finish(api, &mut s).await
    }
}

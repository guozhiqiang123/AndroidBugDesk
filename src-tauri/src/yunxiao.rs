use crate::model::*;
use crate::validation::native_target;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client, Method,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub struct Api {
    client: Client,
    pub profile: Profile,
    base: String,
    oauth: Option<std::sync::Arc<crate::oauth::OAuthSession>>,
}
impl Api {
    pub fn new(profile: Profile, token: &str) -> Result<Self> {
        Self::with_base(profile, token, "https://openapi-rdc.aliyuncs.com")
    }
    pub(crate) fn with_base(profile: Profile, token: &str, base: &str) -> Result<Self> {
        profile.validate()?;
        let mut headers = HeaderMap::new();
        let mut value = HeaderValue::from_str(token).map_err(|_| "Token 格式错误")?;
        value.set_sensitive(true);
        headers.insert("x-yunxiao-token", value);
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "网络客户端初始化失败")?;
        Ok(Self {
            client,
            profile,
            base: base.into(),
            oauth: None,
        })
    }
    pub(crate) fn with_oauth(
        mut self,
        session: Option<std::sync::Arc<crate::oauth::OAuthSession>>,
    ) -> Self {
        self.oauth = session;
        self
    }
    fn url(&self, path: &str) -> String {
        format!(
            "{}/oapi/v1/projex/organizations/{}{}",
            self.base, self.profile.organization_id, path
        )
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(Value, HeaderMap)> {
        self.request_url(method, self.url(path), body).await
    }
    async fn request_url(
        &self,
        method: Method,
        url: String,
        body: Option<Value>,
    ) -> Result<(Value, HeaderMap)> {
        let readonly = method == Method::GET || url.ends_with(":search");
        for attempt in 0..3 {
            let mut req = self.client.request(method.clone(), &url);
            if let Some(session) = &self.oauth {
                let mut token = session.access_token().await?;
                let mut value =
                    HeaderValue::from_str(&token).map_err(|_| "云效授权令牌格式错误")?;
                value.set_sensitive(true);
                req = req.header("x-yunxiao-token", value);
                let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| "云效授权令牌格式错误")?;
                bearer.set_sensitive(true);
                req = req.header("Authorization", bearer);
                use zeroize::Zeroize;
                token.zeroize();
            }
            if let Some(b) = &body {
                req = req.json(b);
            }
            let response = req.send().await.map_err(|_| {
                "网络请求未获得确定响应；写入结果需要核对，禁止直接重试创建".to_string()
            })?;
            let status = response.status();
            if readonly && attempt < 2 && (status.as_u16() == 429 || status.is_server_error()) {
                tokio::time::sleep(std::time::Duration::from_millis(300 * (1 << attempt))).await;
                continue;
            }
            if !status.is_success() {
                return Err(Self::status_error(status.as_u16()));
            }
            let headers = response.headers().clone();
            let bytes = response
                .bytes()
                .await
                .map_err(|_| "响应读取失败，结果需核对")?;
            if bytes.len() > 5_000_000 {
                return Err("接口响应超出安全体积，请检查分页配置".into());
            }
            let data = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).map_err(|_| "接口未返回预期 JSON；写入结果需核对")?
            };
            return Ok((data, headers));
        }
        Err("只读接口重试次数已用尽".into())
    }
    fn status_error(status: u16) -> String {
        match status {
            401 => "[授权失效] 云效授权失效，请重新输入个人 PAT".into(),
            403 => "云效权限不足，请检查项目访问及 PAT 对应资源权限".into(),
            429 => "云效限流，请稍后重试；写入需先核对结果".into(),
            _ => format!("云效接口返回 HTTP {status}，请保留草稿并检查字段、附件及权限"),
        }
    }
    pub async fn list(&self, path: &str, paged: bool) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let mut page = 1usize;
        for _ in 0..1000 {
            let url = if paged {
                format!(
                    "{path}{}page={page}&perPage=100",
                    if path.contains('?') { "&" } else { "?" }
                )
            } else {
                path.into()
            };
            let (data, headers) = self.request(Method::GET, &url, None).await?;
            let rows = data.as_array().ok_or("列表接口格式变化，请核对契约")?;
            all.extend(rows.iter().cloned());
            if paged && has_more(&headers, page, rows.len(), 100)? {
                page += 1;
                continue;
            }
            return Ok(all);
        }
        Err("分页数量超过限制，未完成读取".into())
    }
    pub async fn verify_user(&self) -> Result<crate::workspace::UserIdentity> {
        let (value, _) = self
            .request_url(
                Method::GET,
                format!("{}/oapi/v1/platform/user", self.base),
                None,
            )
            .await
            .map_err(|e| {
                if e.contains("权限不足") {
                    "令牌缺少组织管理「用户」只读权限，无法校验登录；请调整个人令牌权限".into()
                } else {
                    e
                }
            })?;
        let id = string(&value, "id");
        encode_id(&id).map_err(|_| "用户校验响应缺少有效用户 ID")?;
        // GetUserByToken documents nickName but does not promise an avatar.
        // Use an avatar only if this verified user's response actually includes it.
        let avatar_url = ["avatarUrl", "avatar"]
            .iter()
            .map(|key| string(&value, key))
            .find(|url| !url.trim().is_empty());
        let avatar_png = match avatar_url.and_then(|url| trusted_avatar_url(&url)) {
            Some(url) => fetch_avatar_png(url).await,
            None => None,
        };
        Ok(crate::workspace::UserIdentity {
            id,
            name: string(&value, "name"),
            nick_name: string(&value, "nickName"),
            avatar_png,
            last_organization: string(&value, "lastOrganization"),
        })
    }
    pub async fn organizations(&self) -> Result<Vec<OptionItem>> {
        let (value, _) = self
            .request_url(
                Method::GET,
                format!("{}/oapi/v1/platform/organizations", self.base),
                None,
            )
            .await?;
        options(
            value.as_array().ok_or("组织列表格式变化")?.clone(),
            "id",
            "name",
        )
    }
    pub async fn projects(&self) -> Result<Vec<OptionItem>> {
        let mut projects = Vec::new();
        for page in 1..=1000 {
            let (value, headers) = self
                .request(
                    Method::POST,
                    "/projects:search",
                    Some(json!({
                        "page":page,"perPage":100,"orderBy":"gmtCreate","sort":"desc"
                    })),
                )
                .await?;
            let rows = value.as_array().ok_or("项目列表格式变化")?;
            projects.extend(rows.iter().cloned());
            if !has_more(&headers, page, rows.len(), 100)? {
                return options(projects, "id", "name");
            }
        }
        Err("项目列表读取超出分页限制，未完整读取".into())
    }
    pub async fn history(
        &self,
        user_id: &str,
        page: usize,
    ) -> Result<crate::workspace::HistoryPage> {
        self.history_filtered(
            user_id,
            page,
            &crate::history::HistoryFilters::default(),
            None,
            &std::collections::HashMap::new(),
        )
        .await
    }
    pub async fn history_filtered(
        &self,
        user_id: &str,
        start_page: usize,
        filters: &crate::history::HistoryFilters,
        metadata: Option<&Metadata>,
        known_kinds: &std::collections::HashMap<String, String>,
    ) -> Result<crate::workspace::HistoryPage> {
        encode_id(user_id)?;
        filters.validate()?;
        if !(1..=1000).contains(&start_page) {
            return Err("历史记录页码不正确".into());
        }
        // Stop on a matching page or after three empty pages. Never crawl the
        // entire project automatically; page is the consumed server cursor.
        for page in start_page..=(start_page + 2).min(1000) {
            let (value, headers) = self
            .request(
                Method::POST,
                "/workitems:search",
                Some(json!({
                    "category":"Bug","spaceId":self.profile.project_id,"spaceType":"Project",
                    "page":page,"perPage":50,"orderBy":"gmtCreate","sort":"desc",
                    "conditions":serde_json::to_string(&filters.conditions(user_id)).map_err(|_| "历史筛选条件格式错误")?
                })),
            )
            .await?;
            let rows = value.as_array().ok_or("历史记录列表格式变化")?;
            if rows.len() > 50 {
                return Err("云效历史返回超出请求页大小，停止展示".into());
            }
            let mut items = Vec::new();
            for row in rows {
                let id = string(row, "id");
                encode_id(&id).map_err(|_| "历史记录缺少有效工作项 ID")?;
                // A malformed/overbroad response must not leak another project's/user's records.
                if string(row, "space.id") != self.profile.project_id
                    || string(row, "creator.id") != user_id
                    || string(row, "categoryId") != "Bug"
                {
                    return Err("云效历史结果与当前用户/项目/Bug范围不一致，停止展示".into());
                }
                if !filters.matches(row, metadata, known_kinds) {
                    continue;
                }
                items.push(crate::workspace::HistoryItem {
                    id,
                    identifier: string(row, "serialNumber"),
                    title: string(row, "subject"),
                    status: if string(row, "status.displayName").is_empty() {
                        string(row, "status.name")
                    } else {
                        string(row, "status.displayName")
                    },
                    created_at: string(row, "gmtCreate"),
                    updated_at: string(row, "gmtModified"),
                });
            }
            let has_more = has_more(&headers, page, rows.len(), 50)?;
            if page == 1000 && has_more {
                return Err("历史读取已达到分页上限，请缩小筛选范围".into());
            }
            if !items.is_empty() || !has_more || page == start_page + 2 {
                return Ok(crate::workspace::HistoryPage {
                    items,
                    page,
                    has_more,
                });
            }
        }
        Err("历史记录分页未完成".into())
    }
    pub async fn metadata(&self, type_id: &str) -> Result<Metadata> {
        let path = format!("/projects/{}", self.profile.project_id);
        let types = options(
            self.list(&format!("{path}/workitemTypes?category=Bug"), false)
                .await?,
            "id",
            "name",
        )?;
        let selected = if type_id.is_empty() {
            types.first().ok_or("项目没有可用的 Bug 类型")?.id.clone()
        } else {
            type_id.into()
        };
        if !types.iter().any(|v| v.id == selected) {
            return Err("所选 Bug 类型不属于当前项目".into());
        }
        let raw = self
            .list(&format!("{path}/workitemTypes/{selected}/fields"), false)
            .await?;
        let mut fields = raw
            .into_iter()
            .map(|v| {
                Ok(Field {
                    id: string(&v, "id"),
                    name: string(&v, "name"),
                    format: string(&v, "format"),
                    field_type: string(&v, "type"),
                    required: bool_at(&v, "required"),
                    show_when_create: v["showWhenCreate"].as_bool().unwrap_or(true),
                    option_error: None,
                    options: options(
                        v["options"].as_array().cloned().unwrap_or_default(),
                        "id",
                        "displayValue",
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        for field in &mut fields {
            if !crate::submission_fields::excluded_field(field, &self.profile)
                && !crate::submission_fields::hidden_optional_field(field, &self.profile)
                && (field.format == "tag"
                    || crate::validation::native_target(field, &self.profile) == Some("labels"))
                && field.options.is_empty()
            {
                match self.list(&format!("{path}/labels"), true).await {
                    Ok(labels) => field.options = options(labels, "id", "name")?,
                    Err(error) if error.contains("[授权失效]") => return Err(error),
                    Err(_) => {
                        field.option_error = Some(
                            "标签列表未能读取，请检查标签只读权限或稍后刷新字段；其他字段仍可填写"
                                .into(),
                        )
                    }
                }
            }
        }
        let members = options(
            self.list(&format!("{path}/members"), true).await?,
            "userId",
            "userName",
        )?;
        let versions = if self.profile.enable_versions {
            options(
                self.list(&format!("{path}/versions"), true).await?,
                "id",
                "name",
            )?
        } else {
            vec![]
        };
        let sprints = if self.profile.enable_sprints {
            options(
                self.list(&format!("{path}/sprints"), true).await?,
                "id",
                "name",
            )?
        } else {
            vec![]
        };
        Ok(Metadata {
            profile: self.profile.clone(),
            type_id: selected,
            types,
            fields,
            members,
            versions,
            sprints,
            fetched_at: now(),
        })
    }
    pub async fn create(&self, body: Value) -> Result<String> {
        let (v, _) = self.request(Method::POST, "/workitems", Some(body)).await?;
        let id = string(&v, "id");
        if id.is_empty() {
            Err("创建响应缺少真实工作项 ID，必须核对远端".into())
        } else {
            Ok(id)
        }
    }
    pub async fn get(&self, id: &str) -> Result<Value> {
        Ok(self
            .request(Method::GET, &format!("/workitems/{}", encode_id(id)?), None)
            .await?
            .0)
    }
    pub async fn update_description(&self, id: &str, description: &str) -> Result<()> {
        self.request(
            Method::PUT,
            &format!("/workitems/{}", encode_id(id)?),
            Some(json!({"description":description,"formatType":"MARKDOWN"})),
        )
        .await?;
        Ok(())
    }
    pub async fn attachments(&self, id: &str) -> Result<Vec<Value>> {
        self.list(&format!("/workitems/{}/attachments", encode_id(id)?), false)
            .await
    }
    pub async fn upload(&self, id: &str, a: &Attachment, name: &str) -> Result<String> {
        let part = reqwest::multipart::Part::file(&a.path)
            .await
            .map_err(|_| "无法读取附件")?
            .file_name(name.to_string())
            .mime_str(&a.mime)
            .map_err(|_| "附件 MIME 错误")?;
        let response = self
            .client
            .post(self.url(&format!("/workitems/{}/attachments", encode_id(id)?)))
            .multipart(reqwest::multipart::Form::new().part("file", part))
            .send()
            .await
            .map_err(|_| "附件上传未获得确定响应，需回读核对")?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(format!(
                "{}{}",
                if [400, 401, 403, 413, 415, 422].contains(&status) {
                    "[未写入] "
                } else {
                    ""
                },
                Self::status_error(status)
            ));
        }
        let data: Value = response
            .json()
            .await
            .map_err(|_| "附件响应丢失，需回读核对")?;
        let id = string(&data, "id");
        if id.is_empty() {
            Err("附件响应缺少 ID，需核对".into())
        } else {
            Ok(id)
        }
    }
    pub async fn matches_download(&self, row: &Value, a: &Attachment) -> Result<bool> {
        let url = string(row, "url");
        let parsed = reqwest::Url::parse(&url).map_err(|_| "附件缺少可用下载地址")?;
        if parsed.scheme() != "https"
            && !(self.base.starts_with("http://127.0.0.1:")
                && parsed.host_str() == Some("127.0.0.1"))
        {
            return Err("附件下载必须使用 HTTPS".into());
        }
        // Download signed URLs with a separate client: never forward the personal token to attachment hosts.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "下载客户端初始化失败")?;
        let mut r = client
            .get(parsed)
            .send()
            .await
            .map_err(|_| "附件下载核对失败")?;
        if !r.status().is_success() {
            return Err("附件下载核对未成功".into());
        }
        let mut hash = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = r.chunk().await.map_err(|_| "附件下载中断")? {
            size += chunk.len() as u64;
            if size > a.size {
                return Ok(false);
            }
            hash.update(chunk);
        }
        Ok(size == a.size && hex::encode(hash.finalize()) == a.sha256)
    }
    pub async fn candidates(&self, title: &str) -> Result<Vec<Value>> {
        let mut result = Vec::new();
        for page in 1..=1000 {
            let (v,h)=self.request(Method::POST,"/workitems:search",Some(json!({"spaceId":self.profile.project_id,"category":"Bug","page":page,"perPage":100,"orderBy":"gmtCreate","sort":"desc","conditions":serde_json::to_string(&json!({"conditionGroups":[[{"fieldIdentifier":"subject","operator":"CONTAINS","value":[title],"toValue":null,"className":"string","format":"input"}]]})).unwrap()}))).await?;
            let rows = v.as_array().ok_or("搜索返回格式变化")?;
            result.extend(rows.iter().cloned());
            let next = h
                .get("x-next-page")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if next.is_empty() && rows.len() < 100 {
                return Ok(result);
            }
        }
        Err("搜索不完整，保持创建结果不确定".into())
    }
}
fn has_more(headers: &HeaderMap, page: usize, count: usize, per_page: usize) -> Result<bool> {
    // The real service emits x-next-page even at/after the last page.
    if let Some(total) = headers.get("x-total-pages").and_then(|v| v.to_str().ok()) {
        let total = total.parse::<usize>().map_err(|_| "接口总页数不正确")?;
        if page >= total {
            return Ok(false);
        }
    }
    if let Some(next) = headers
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        let n = next.parse::<usize>().map_err(|_| "接口分页参数不正确")?;
        if n <= page {
            return Err("接口分页循环，停止读取".into());
        }
        if n != page + 1 {
            return Err("接口分页不连续，停止读取".into());
        }
        return Ok(true);
    }
    if let Some(total) = headers.get("x-total-pages").and_then(|v| v.to_str().ok()) {
        return Ok(page < total.parse::<usize>().map_err(|_| "接口总页数不正确")?);
    }
    // No documented completion header: offer another page for a full response.
    Ok(count == per_page)
}
pub(crate) fn encode_id(id: &str) -> Result<&str> {
    if regex::Regex::new(r"^[A-Za-z0-9_-]{1,128}$")
        .unwrap()
        .is_match(id)
    {
        Ok(id)
    } else {
        Err("云效 ID 不正确".into())
    }
}
fn options(values: Vec<Value>, id_key: &str, name_key: &str) -> Result<Vec<OptionItem>> {
    let mut result = Vec::new();
    for v in values {
        let id = string(&v, id_key);
        if id.is_empty() {
            return Err("元数据缺少选项 ID，停止提交".into());
        }
        if !result.iter().any(|o: &OptionItem| o.id == id) {
            result.push(OptionItem {
                id,
                name: if string(&v, name_key).is_empty() {
                    string(&v, "name")
                } else {
                    string(&v, name_key)
                },
            });
        }
    }
    Ok(result)
}
pub fn create_payload(d: &Value, m: &Metadata, description: String) -> Result<Value> {
    let mut body = json!({"subject":d["title"],"spaceId":m.profile.project_id,"workitemTypeId":m.type_id,"assignedTo":d["assignedTo"],"description":description,"formatType":"MARKDOWN"});
    if !string(d, "sprint").is_empty() {
        body["sprint"] = d["sprint"].clone();
    }
    if d["versions"].as_array().is_some_and(|a| !a.is_empty()) {
        body["versions"] = d["versions"].clone();
    }
    let mut custom = serde_json::Map::new();
    for f in &m.fields {
        if crate::submission_fields::excluded_field(f, &m.profile) {
            if f.required || m.profile.required_field_ids.contains(&f.id) {
                return Err(format!(
                    "云效项目将已移除字段「{}」设为必填，请调整项目配置后提交",
                    f.name
                ));
            }
            continue;
        }
        if crate::submission_fields::hidden_optional_field(f, &m.profile) {
            continue;
        }
        let v = d["customFields"][&f.id].clone();
        if v.is_null() || v == "" || v.as_array().is_some_and(|a| a.is_empty()) {
            continue;
        }
        if let Some(target) = native_target(f, &m.profile) {
            if ["subject", "description", "assignedTo", "sprint", "versions"].contains(&target) {
                continue;
            }
            body[target] = v;
        } else if ["CustomField", "SystemCustomField"].contains(&f.field_type.as_str()) {
            custom.insert(
                f.id.clone(),
                Value::String(
                    v.as_array()
                        .map(|a| {
                            a.iter()
                                .map(|v| v.as_str().unwrap_or(""))
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_else(|| v.as_str().unwrap_or("").into()),
                ),
            );
        } else {
            return Err(format!("字段 {} 缺少映射", f.name));
        }
    }
    body["customFieldValues"] = Value::Object(custom);
    Ok(body)
}

// The renderer receives normalized image bytes; it never loads remote URLs or PATs.
fn trusted_avatar_url(value: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("tcs-devops.aliyuncs.com")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with("/thumbnail/"))
    .then_some(url)
}

async fn fetch_avatar_png(url: reqwest::Url) -> Option<Vec<u8>> {
    // A distinct client ensures the personal token cannot reach an image host.
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let mut response = client.get(url).send().await.ok()?;
    if !response.status().is_success() || response.content_length().unwrap_or(0) > 512_000 {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 512_000 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    normalize_avatar(&bytes)
}

fn normalize_avatar(bytes: &[u8]) -> Option<Vec<u8>> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(16_777_216);
    reader.limits(limits);
    let avatar = reader.decode().ok()?.thumbnail(128, 128);
    let mut output = std::io::Cursor::new(Vec::new());
    avatar.write_to(&mut output, image::ImageFormat::Png).ok()?;
    Some(output.into_inner())
}

#[cfg(test)]
mod user_profile_tests {
    use super::*;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn avatar_urls_reject_non_cdn_urls_credentials_and_ports() {
        assert!(
            trusted_avatar_url("https://tcs-devops.aliyuncs.com/thumbnail/test/w/100/h/100")
                .is_some()
        );
        for url in [
            "http://tcs-devops.aliyuncs.com/thumbnail/a",
            "https://127.0.0.1/thumbnail/a",
            "https://tcs-devops.aliyuncs.com.evil.test/thumbnail/a",
            "https://user:secret@tcs-devops.aliyuncs.com/thumbnail/a",
            "https://tcs-devops.aliyuncs.com:9443/thumbnail/a",
            "file:///tmp/avatar.png",
            "https://tcs-devops.aliyuncs.com/other/a",
        ] {
            assert!(trusted_avatar_url(url).is_none(), "{url}");
        }
    }

    #[tokio::test]
    async fn avatar_download_has_no_token_and_rejects_redirects_and_invalid_images() {
        let server = MockServer::start().await;
        let image = image::DynamicImage::new_rgba8(256, 256);
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        Mock::given(method("GET"))
            .and(path("/avatar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png.into_inner()))
            .mount(&server)
            .await;
        let bytes = fetch_avatar_png(format!("{}/avatar", server.uri()).parse().unwrap())
            .await
            .unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 128);
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("x-yunxiao-token"));
        assert!(!requests[0].headers.contains_key("authorization"));
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/avatar", server.uri())),
            )
            .mount(&server)
            .await;
        assert!(
            fetch_avatar_png(format!("{}/redirect", server.uri()).parse().unwrap())
                .await
                .is_none()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert!(normalize_avatar(b"<svg><script>bad</script></svg>").is_none());
        assert!(normalize_avatar(b"not an image").is_none());
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 512_001]))
            .mount(&server)
            .await;
        assert!(
            fetch_avatar_png(format!("{}/large", server.uri()).parse().unwrap())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn documented_nickname_is_retained_and_missing_avatar_needs_no_second_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/oapi/v1/platform/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"user-a", "name":"企业姓名", "nickName":"测试昵称", "lastOrganization":"org-a"})))
            .expect(1).mount(&server).await;
        let api = Api::with_base(
            crate::workspace::empty_profile(),
            "mock-token",
            &server.uri(),
        )
        .unwrap();
        let user = api.verify_user().await.unwrap();
        assert_eq!(user.id, "user-a");
        assert_eq!(user.nick_name, "测试昵称");
        assert_eq!(user.name, "企业姓名");
        assert!(user.avatar_png.is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

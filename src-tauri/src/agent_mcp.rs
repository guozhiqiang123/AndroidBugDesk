use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, schemars, tool, tool_handler,
    tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftIdInput {
    #[schemars(description = "Android Bug Desk 草稿 UUID")]
    draft_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevisionInput {
    #[schemars(description = "Android Bug Desk 草稿 UUID")]
    draft_id: String,
    #[schemars(description = "最近一次 get_draft 返回的 revision")]
    expected_revision: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPatchInput {
    occurred_at: Option<String>,
    source_note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrequencyPatchInput {
    first_discovery: Option<bool>,
    attempts: Option<String>,
    occurrences: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftPatchInput {
    jenkins_package_url: Option<String>,
    title: Option<String>,
    kind: Option<String>,
    bug_page: Option<String>,
    #[schemars(description = "Markdown 复现条件")]
    preconditions: Option<String>,
    #[schemars(description = "Markdown 复现步骤")]
    steps: Option<String>,
    #[schemars(description = "Markdown 期望表现")]
    expected: Option<String>,
    #[schemars(description = "Markdown 问题表现")]
    actual: Option<String>,
    event: Option<EventPatchInput>,
    frequency: Option<FrequencyPatchInput>,
    workitem_type_id: Option<String>,
    assigned_to: Option<String>,
    sprint: Option<String>,
    versions: Option<Vec<String>>,
    custom_fields: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchDraftInput {
    draft_id: String,
    expected_revision: String,
    patch: DraftPatchInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnvironmentCaptureInput {
    draft_id: String,
    expected_revision: String,
    #[schemars(description = "list_devices 返回且状态为 device 的设备序列号")]
    serial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceCaptureInput {
    draft_id: String,
    expected_revision: String,
    serial: String,
    #[schemars(
        description = "补采项，可选 environment/logs/screenshot/ui/app/activities/display/locale/crash/exit/memory/frames/cpu/network/storage/bugreport"
    )]
    items: Vec<String>,
    retry_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportAttachmentInput {
    draft_id: String,
    expected_revision: String,
    #[schemars(description = "用户明确提供或当前任务生成的普通文件绝对路径")]
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveAttachmentInput {
    draft_id: String,
    expected_revision: String,
    attachment_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct AgentMcpServer;

#[tool_router]
impl AgentMcpServer {
    #[tool(
        description = "读取 Android Bug Desk 当前登录、项目、包名、ADB 和草稿摘要。开始任何自动提 Bug 工作流时先调用。"
    )]
    async fn workspace_status(&self) -> Result<CallToolResult, McpError> {
        forward("workspace_status", json!({})).await
    }

    #[tool(description = "列出 Android Bug Desk 当前 ADB 可见设备及连接状态。")]
    async fn list_devices(&self) -> Result<CallToolResult, McpError> {
        forward("list_devices", json!({})).await
    }

    #[tool(description = "在当前已登录账号和当前项目中新建一个本机 Bug 草稿。")]
    async fn create_draft(&self) -> Result<CallToolResult, McpError> {
        forward("create_draft", json!({})).await
    }

    #[tool(description = "读取当前项目中的一个草稿和 revision；受管附件路径会被隐藏。")]
    async fn get_draft(
        &self,
        Parameters(input): Parameters<DraftIdInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("get_draft", input).await
    }

    #[tool(
        description = "按字段白名单更新 Bug 草稿。必须传最近一次读取到的 revision；冲突时重新读取，不能覆盖。"
    )]
    async fn patch_draft(
        &self,
        Parameters(input): Parameters<PatchDraftInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("patch_draft", input).await
    }

    #[tool(description = "执行与客户端提交前相同的 Rust 字段和附件校验。")]
    async fn validate_draft(
        &self,
        Parameters(input): Parameters<DraftIdInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("validate_draft", input).await
    }

    #[tool(description = "异步采集一份受限的设备环境快照并加入草稿附件，返回 jobId。")]
    async fn capture_environment(
        &self,
        Parameters(input): Parameters<EnvironmentCaptureInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("capture_environment", input).await
    }

    #[tool(description = "按 Android Bug Desk 固定白名单异步补采证据，返回 jobId。")]
    async fn capture_evidence(
        &self,
        Parameters(input): Parameters<EvidenceCaptureInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("capture_evidence", input).await
    }

    #[tool(description = "读取某草稿的补采任务进度、逐项结果和最新安全草稿。")]
    async fn capture_status(
        &self,
        Parameters(input): Parameters<DraftIdInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("capture_status", input).await
    }

    #[tool(
        description = "把用户明确提供或当前任务生成的普通文件复制到草稿受管附件目录。不能导入目录或符号链接。"
    )]
    async fn import_attachment(
        &self,
        Parameters(input): Parameters<ImportAttachmentInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("import_attachment", input).await
    }

    #[tool(description = "列出草稿附件的安全元数据，不返回本机受管路径。")]
    async fn list_attachments(
        &self,
        Parameters(input): Parameters<DraftIdInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("list_attachments", input).await
    }

    #[tool(description = "按附件 ID 从草稿移除附件，沿用关联回执成组删除规则。")]
    async fn remove_attachment(
        &self,
        Parameters(input): Parameters<RemoveAttachmentInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("remove_attachment", input).await
    }

    #[tool(
        description = "生成最终提交前的结构化预览和校验结果。此工具不会创建云效 Bug；用户必须回到客户端确认提交。"
    )]
    async fn prepare_submission(
        &self,
        Parameters(input): Parameters<RevisionInput>,
    ) -> Result<CallToolResult, McpError> {
        forward_value("prepare_submission", input).await
    }
}

#[tool_handler(
    name = "android-bug-desk",
    instructions = "Prepare Android Bug Desk drafts and evidence. Never invent missing facts. Final Yunxiao submission requires the user to confirm in the desktop client."
)]
impl ServerHandler for AgentMcpServer {}

async fn forward_value<T: Serialize>(method: &str, input: T) -> Result<CallToolResult, McpError> {
    match serde_json::to_value(input) {
        Ok(params) => forward(method, params).await,
        Err(error) => Ok(CallToolResult::structured_error(
            json!({"message":format!("工具参数序列化失败：{error}")}),
        )),
    }
}

async fn forward(method: &str, params: Value) -> Result<CallToolResult, McpError> {
    match crate::agent_gateway::request(method, params).await {
        Ok(result) => Ok(CallToolResult::structured(result)),
        Err(error) => Ok(CallToolResult::structured_error(json!({"message":error}))),
    }
}

pub fn run() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("无法启动 MCP runtime：{error}"))?;
    runtime.block_on(async {
        let service = AgentMcpServer
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| format!("无法启动 MCP stdio：{error}"))?;
        service
            .waiting()
            .await
            .map_err(|error| format!("MCP stdio 异常结束：{error}"))?;
        Ok(())
    })
}

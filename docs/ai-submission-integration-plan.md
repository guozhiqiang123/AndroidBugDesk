# AI 自动提 Bug 集成计划

状态：首版已实施，待 Windows/Linux 发布构建与真机验收
分支：`codex/ai-bug-submission`
基线：`v0.3.7` / `545d39d`
日期：2026-10-22

## 1. 决策

保留 Android Bug Desk 客户端，将它作为设备、凭据、草稿、证据和云效提交的唯一执行与安全边界；在客户端之上增加受限的本地 Agent Gateway，再用独立的 stdio MCP bridge 暴露结构化工具。Skill 只负责提 Bug 的步骤、字段提取、证据选择和缺项处理，不复制 ADB、附件、云效 API 或提交恢复代码。

目标结构：

```text
Codex / 其他支持 MCP 的 AI
        │
        │ stdio MCP（官方 Rust SDK）
        ▼
android-bug-desk --mcp-bridge
        │
        │ 当前用户本机 IPC + 每次启动随机能力令牌
        ▼
Android Bug Desk Agent Gateway
        │
        ▼
现有 AppState / Store / ADB / Evidence / Yunxiao API
        ▲
        │
React / Tauri 桌面界面
```

不采用“完全抛弃客户端、只做 Skill”的方案。纯 Skill 无法可靠承担 USB/ADB 生命周期、系统安全存储、跨平台文件复制、证据哈希、SQLite 事务、提交幂等恢复和人工预览。已有云效 Bug 读取仍适合由 `yunxiao-bug-reader` 等只读 Skill 完成；新 Bug 的设备取证和提交由本客户端负责。

## 2. 现状与改造边界

当前实现已经具备以下可复用基础：

- `commands::AppState` 是账号、项目、SQLite、ADB、采集器和提交锁的进程内权威状态。
- 草稿、采集任务和提交状态保存在 `Store`；云效提交由 `Submitter` 的持久状态机防重和恢复。
- 附件导入会复制到应用管理目录并校验类型、大小和 SHA256；补采使用固定 ADB 参数白名单。
- Tauri 命令目前同时承担 UI 边界和业务编排，部分函数已经存在 `&AppState` 形式的内部入口，但尚无统一的 AI 应用服务。
- 提交预览由界面在保存和 Rust 二次校验后展示，最终“提交 Bug”是明确的用户动作。

本次改造遵守以下边界：

1. PAT、OAuth token 和授权请求头不通过 MCP 返回，也不写入 Skill、草稿、日志或工具结果。
2. MCP 不提供任意 shell、任意 ADB 命令、任意 HTTP 请求、任意 SQLite 查询或任意文件读取。
3. 工具只操作当前已登录账号、当前项目和其草稿；项目切换继续由客户端负责。
4. AI 可以准备完整草稿和证据，但首版不能直接创建远端 Bug。最终提交仍在客户端预览弹窗中由用户点击。
5. 本地 IPC 只允许当前机器上的 bridge 访问，使用每次客户端启动重新生成的随机能力令牌；客户端关闭后旧令牌失效。
6. 所有写操作使用草稿 `updatedAt` 作为乐观锁 revision，调用方必须传 `expectedRevision`，避免 AI 和界面互相覆盖。

## 3. 本地通信与生命周期

### 3.1 Agent Gateway

Gateway 运行在现有 Tauri 主进程内，因此直接复用同一个 `AppState`，不会出现第二套登录态、内存锁或采集器。

- macOS/Linux：应用数据目录中的 Unix domain socket。
- Windows：当前用户会话的 named pipe。
- 描述文件只记录协议版本、端点、进程号、启动时间和随机令牌；Unix 权限设为 `0600`，应用数据目录保持 `0700`。
- 设置页新增“允许本机 AI 接入”开关。关闭时不保留可连接端点；关闭或退出应用时停止监听并删除描述文件。
- 每条请求限制大小、设置读取超时，并带协议版本、请求 ID 和能力令牌。

### 3.2 MCP bridge

同一可执行文件支持 `--mcp-bridge` 启动模式。该模式不初始化 Tauri 窗口，只启动 stdio MCP server，读取 Gateway 描述文件并转发白名单工具。这样安装包不需要额外 sidecar，也能复用 macOS、Windows 和 Linux 的现有发布流程。

bridge 使用 Model Context Protocol 官方 Rust SDK `rmcp`，stdout 仅输出 MCP 协议，诊断信息写 stderr。客户端未启动、未登录、未开启 AI 接入或端点已过期时，工具返回可执行的中文错误，不自行启动应用或读取凭据。

参考：

- 官方 Rust SDK：<https://github.com/modelcontextprotocol/rust-sdk>
- 本地 stdio transport：<https://ts.sdk.modelcontextprotocol.io/server>

## 4. 首版工具契约

| 工具                  | 类型      | 作用                                   | 约束                                             |
| --------------------- | --------- | -------------------------------------- | ------------------------------------------------ |
| `workspace_status`    | 只读      | 当前登录、项目、包名、ADB 和草稿摘要   | 不返回 token、附件本地路径                       |
| `list_devices`        | 只读      | 列出 ADB 设备和状态                    | 使用用户已选 ADB                                 |
| `create_draft`        | 写        | 在当前项目创建草稿                     | 复用项目包名门槛                                 |
| `get_draft`           | 只读      | 获取当前范围内草稿及 revision          | 附件只返回安全元数据                             |
| `patch_draft`         | 写        | 更新允许 AI 填写的字段                 | 拒绝未知字段、附件数组、来源、设备身份和本地路径 |
| `validate_draft`      | 只读      | 执行与提交前相同的 Rust 校验           | 返回字段级问题及最新 revision                    |
| `capture_environment` | 异步写    | 采集设备环境快照                       | 设备、项目、包名和草稿范围必须一致               |
| `capture_evidence`    | 异步写    | 按现有白名单补采证据                   | 只接受 `post_capture::ITEMS` 中的项目            |
| `capture_status`      | 只读      | 获取采集进度、失败项和最新草稿         | 不返回附件本地路径                               |
| `import_attachment`   | 写        | 将明确路径的普通文件复制进受管附件目录 | 拒绝符号链接、目录、程序/脚本和超限文件          |
| `list_attachments`    | 只读      | 返回名称、类型、大小、哈希、来源和角色 | 不返回受管绝对路径                               |
| `remove_attachment`   | 写        | 按附件 ID 移除草稿附件                 | 沿用成组回执和受管文件清理规则                   |
| `prepare_submission`  | 只读/准备 | 校验并返回提交预览摘要                 | 不调用云效创建接口；提示用户在客户端确认提交     |

首版不暴露 `commit_submission`。后续如确需无人值守提交，应另行增加项目级授权、一次性确认、幂等键、审计记录和可撤销策略，不能复用普通工具调用默认为授权。

## 5. 草稿写入与冲突处理

`patch_draft` 采用结构化 patch，而不是让 AI 回传整个草稿。首版允许修改：

- `jenkinsPackageUrl`
- `title`
- `kind`
- `bugPage`
- `preconditions`
- `steps`
- `expected`
- `actual`
- `event.occurredAt`
- `event.sourceNote`
- `frequency.firstDiscovery/attempts/occurrences`
- `workitemTypeId/assignedTo/sprint/versions/customFields`

调用流程：读取当前草稿 → 比对 `expectedRevision` → 合并白名单字段 → 复用现有保存、项目包名、Schema 和凭据泄漏校验 → 写入新的 `updatedAt`。revision 不一致时返回最新 revision 和“先重新读取”的冲突错误，不做最后写入覆盖。

Gateway 成功写入后向 renderer 发送 `agent-draft-updated` 事件。新建提交页收到事件后刷新当前状态；若当前字段有未保存编辑，界面保留本地内容并提示用户处理冲突，不静默覆盖。

## 6. Skill 设计

仓库内提供 `android-bug-submission` Skill，保持简短，只描述可重复工作流：

1. 先调用 `workspace_status`，确认登录、项目和包名。
2. 从用户描述中提取事实；缺少的信息保持空值，不猜测设备、版本、页面、时间或复现结果。
3. 创建或读取草稿，使用 revision 写入字段。
4. 仅在用户明确连接设备并需要取证时列设备、采集环境或补采证据。
5. 仅导入用户明确提供或本次任务生成的文件。
6. 校验草稿，逐项修正可修正的问题。
7. 调用 `prepare_submission` 后，把缺项和预览交给用户；提示在 Android Bug Desk 中确认并提交。

Skill 不包含 PAT、组织 ID、项目 ID、包名、个人信息或机器绝对路径。工具不可用时不改走云效网页自动提交，也不调用通用 shell 模拟 ADB。

## 7. 实施阶段

### 阶段 A：地基与只读闭环

- 提取 AI 应用服务的安全 DTO、草稿范围检查、脱敏输出和 revision。
- 实现 Gateway 生命周期、能力令牌和跨平台本地 IPC。
- 实现 `--mcp-bridge` 与 `workspace_status`、`get_draft`、`validate_draft`。
- 设置页增加 AI 接入开关和连接状态。

验收：默认关闭；未启动/未登录/项目未配置时给出正确错误；工具结果没有 token 和受管文件路径；现有 Tauri 功能不回归。

### 阶段 B：草稿编辑与界面同步

- 实现 `create_draft`、`patch_draft`、`prepare_submission`。
- 增加乐观锁并覆盖 UI 与 AI 并发写入测试。
- Gateway 写入后通知 renderer 刷新，避免必须重启或切换页面才能看到内容。

验收：AI 可填完一份草稿；并发修改会明确冲突；提交预览内容与界面一致；AI 无法直接提交。

### 阶段 C：设备证据与附件

- 实现设备列表、环境快照、补采启动/状态、附件导入/列出/移除。
- 复用现有白名单、UID 过滤、任务范围、超时、磁盘限制、哈希和成组回执规则。
- 长任务返回 job ID，由调用方轮询，不占用 MCP 请求直到 ADB 完成。

验收：断线、ADB 低版本、命令超时、项目切换和客户端退出均留下真实状态；不会退化为整机无限日志或任意命令。

### 阶段 D：Skill、安装与跨平台验证

- 创建并校验 Skill；补充 Codex/MCP 配置示例和已安装程序路径说明。
- 验证 macOS arm64 本机；由发布工作流验证 Windows x64、macOS x64 和 Linux x64 编译。
- Windows 真机重点验证 named pipe、隐藏窗口模式、ADB 超时和系统代理不影响本地桥接。

验收：Codex 能通过 Skill 完成“创建草稿 → 填写 → 采集 → 加附件 → 校验 → 交用户确认”的完整流程。

## 8. 测试与质量门槛

- 单元测试：DTO 严格反序列化、字段白名单、revision 冲突、脱敏输出、令牌拒绝、端点描述文件权限与清理。
- Rust 集成测试：MCP tool → bridge client → Gateway → `AppState`，使用临时 Store 和模拟状态，不访问真实云效或 USB。
- 现有业务验证：`npm run check`、`cargo fmt --check`、严格 Clippy；涉及设置和新建提交页时增加实际 React 模拟场景。
- 协议验证：启动 `--mcp-bridge`，完成 MCP initialize、tools/list 和代表性 tools/call；stdout 不含诊断噪声。
- 安全检查：结果中搜索 token、`Authorization`、附件绝对路径和本地数据库路径；未知工具和超大请求必须拒绝。

本轮不执行真实云效写入，也不自动提交 Bug。真实 ADB 采集和 Windows 安装验证仍按现有验收门槛单独记录。

## 9. 完成定义

本功能完成需同时满足：

1. 客户端仍是账号、项目、ADB、附件和提交状态的唯一权威实现。
2. 支持 MCP 的 AI 能在客户端运行时完成草稿和证据准备。
3. 人工与 AI 并发编辑不会静默丢失数据。
4. AI 无法读取凭据、执行任意系统命令或绕过当前项目范围。
5. 最终远端创建仍经过客户端提交预览和用户明确点击。
6. macOS 本机及 Windows/Linux 构建检查通过，安装和真实设备验收分别留有记录。

## 10. 本轮实施记录

已完成阶段 A 至 D 的首版代码：

- 新增跨平台 Agent Gateway。macOS/Linux 使用 Unix domain socket，Windows 使用 named pipe；接入默认关闭，启用时生成一次性能力令牌和受保护描述文件，关闭应用或开关后立即失效。
- 同一桌面可执行文件新增 `--mcp-bridge` 模式，使用官方 `rmcp` SDK 提供 13 个 stdio MCP 工具。工具覆盖草稿创建、读取、白名单字段更新、校验、设备列表、环境/补采任务、附件和提交前预览，不提供远端提交。
- 草稿写操作增加 revision 乐观锁；AI 更新通过 Tauri 事件通知界面。界面存在未保存内容时保留人工输入并提示冲突，不静默覆盖。
- 设置页新增“允许本机 AI 接入”开关，并保留原有调试开关。旧设置通过 serde 默认值兼容。
- 新增仓库内 `android-bug-submission` Skill、`agents/openai.yaml` 和 MCP 配置示例。
- 客户端侧栏新增“接入 AI”页。Skill、MCP 示例和说明已编译进二进制并释放到应用本地数据目录，页面生成包含当前安装路径的提示语，使用者可直接复制给 Codex 完成安装和连接验证。

本机验证结果：

- `npm run check`：217 项 Rust 测试通过，3 项需显式真实凭据的只读测试跳过；TypeScript 和仓库脚本检查通过。
- `cargo fmt --check` 与严格 Clippy：通过。
- Agent 专项测试：5 项通过，覆盖严格参数、revision 冲突、路径脱敏、令牌比较和真实本地 IPC 往返/清理。
- React 浏览器集成检查与历史筛选检查：通过；覆盖设置开关和 Tauri 事件模拟。
- MCP 子进程协议检查：完成 initialize、`tools/list` 和 `workspace_status` 错误路径；协议版本 `2025-11-25`，共暴露 13 个工具，stdout 无额外诊断输出。
- Skill `quick_validate.py`：通过。
- macOS 安装验收发现并修复了启用 AI 接入后的启动崩溃：Gateway 现在只在 Tauri 异步运行时内创建 Tokio IPC listener；连续两次启动均生成有效端点，未增加崩溃报告。
- 完整本地模拟已通过“读取工作区 → 新建草稿 → 填写结构化内容 → 导入图片附件 → 校验 → 生成提交预览”，校验缺项为 0，提交策略保持 `prepare-only`，未产生云效写入。验收回执保存在本轮本地打包产物目录。

仍需在发布阶段完成 Windows x64、macOS x64 和 Linux x64 的 CI 构建，并在 Windows 真机验证 named pipe、已安装程序路径和 ADB 长任务。本轮未执行真实云效写入，也未自动提交 Bug。

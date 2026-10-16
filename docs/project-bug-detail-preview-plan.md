# 项目 Bug 完整详情预览计划

## 1. 目标与范围

“项目 Bug”列表中的工作项在应用内打开完整只读详情，减少在客户端与云效网页之间切换。详情体验覆盖开发排查所需的主要信息，同时保留“在云效中打开”作为不支持字段和后续写操作的兜底。

本次包含：

- 标题、编号、状态、负责人、创建者、验证者、参与者、抄送、标签、迭代、创建/更新时间等标准字段。
- 云效返回的全部自定义字段，以名称和值的稳定只读形式展示。
- `MARKDOWN` 与 `RICHTEXT` 两种问题描述，禁止执行远端 HTML、脚本或事件属性。
- 附件列表、图片/文本应用内预览，以及其他文件通过系统应用预览。
- 评论列表，保留作者、时间、置顶和回复层级信息。
- 工作项动态，包括创建、字段修改、状态流转、附件与关联项变化。
- 加载、局部失败、刷新、登录失效、项目切换和过期请求处理。

本次不提供字段编辑、状态流转、评论发布、附件上传或删除。这些属于远端写操作，需要单独设计权限、并发覆盖和失败恢复流程。

## 2. 交互设计

点击列表行或“查看详情”后，在当前“项目 Bug”页面上打开宽幅详情弹窗。复用 `AppDialog` 的关闭按钮、焦点恢复、ESC 和滚动锁定；列表筛选、页码和滚动位置保持不变。

详情结构：

1. 固定标题栏：Bug 编号、标题、状态，以及“刷新”和“在云效中打开”。
2. 摘要区：负责人、创建者、验证者、迭代、标签、创建时间和更新时间。
3. 描述区：根据云效 `formatType` 渲染 Markdown 或经过清理的富文本；空描述显示明确空状态。
4. 字段区：标准补充字段与动态自定义字段采用紧凑键值布局；长值自动换行，不直接展示供应商 JSON。
5. 附件区：显示名称、类型、大小、上传人和时间。PNG/JPEG 与常见 UTF-8 文本可在应用内预览；其他格式调用系统应用。
6. 评论区：按时间展示作者和正文，支持 Markdown/富文本只读渲染；通过缩进和“回复”标识表达父子关系。
7. 动态区：按时间倒序展示操作者、动作、字段名称以及变更前后值。

打开详情时显示骨架/加载状态；失败时保留弹窗并提供重试。附件、评论或动态的非认证失败只影响对应分区，详情主体仍可阅读。任何接口返回授权失效时继续使用全局流程清除会话并回到登录页。

## 3. 数据与模块边界

新增稳定共享契约，React 不直接消费云效原始 JSON：

```text
ProjectBugDetail
  id / identifier / title / description / format
  status / assignedTo / creator / modifier / verifier
  participants[] / trackers[] / labels[] / sprint
  createdAt / updatedAt / statusUpdatedAt / archived
  customFields[]
  attachments[] / comments[] / activities[]
  sectionWarnings[]

ProjectBugAttachment
  id / fileId / name / suffix / size / creator / createdAt

ProjectBugComment
  id / parentId / author / content / format / createdAt / updatedAt / pinned

ProjectBugActivity
  id / eventType / actionType / actor / propertyName
  oldValues[] / newValues[] / relatedResource / occurredAt
```

后端继续放在 `project_bugs.rs`，负责工作区作用域、供应商字段映射和局部错误隔离；`yunxiao.rs` 只增加通用详情附属接口，不加入界面语义。前端新增独立详情组件，`ProjectBugsView` 只负责选中项、缓存和打开/关闭，不把详情状态放进 `main.tsx`。

## 4. 请求、缓存与竞态

- 详情命令接收当前 `projectId` 和工作项内部 `id`，不根据页面编号重新搜索。
- 先读取工作项主体并校验 `space.id = 当前项目`、`categoryId = Bug`，再发布结果。
- 主体、附件、评论和动态使用同一个不可变 `ReadScope`；网络完成后再次核对账号、认证纪元和项目纪元。
- 附属数据并行读取。授权错误提升为整个请求失败；普通权限或兼容错误转为分区警告。
- 前端以工作项 ID 建立当前项目内的短时内存缓存。再次打开先显示缓存，超过两分钟或点击刷新时重新读取；项目/账号切换由页面作用域键销毁缓存。
- 每次详情读取使用递增请求序号，关闭弹窗、切换 Bug、退出登录或切换项目后，迟到响应不能更新当前界面。
- 不把远端详情写入 SQLite，避免把可重取的云效数据变成本地事实来源。

## 5. 内容安全

- Markdown 使用现有只读编辑器 schema 渲染；链接仅允许安全协议，远端图片不由 WebView 直接携带凭据访问。
- 富文本先通过严格清理器删除脚本、样式、表单、iframe、对象、事件属性和危险 URL，再交给只读渲染器。
- 附件下载只接受云效返回且本次详情中存在的附件 ID；点击时重新获取当前临时 URL，避免缓存过期签名。
- 下载使用独立无 PAT 客户端，不向 OSS 或其他附件域名转发云效令牌；只接受 HTTPS、禁止重定向，并限制单文件大小。
- 应用内预览只返回受限大小的 PNG/JPEG 和 UTF-8 文本。其他文件保存到应用临时目录并调用系统预览，不暴露任意路径 IPC。
- 临时附件按账号/项目/工作项/附件隔离，启动时和下载前执行容量/过期清理，限制总量，避免长期占用磁盘。

## 6. 云效接口依据

- [GetWorkitem：获取工作项](https://help.aliyun.com/zh/yunxiao/developer-reference/getworkitem)
- [ListWorkitemAttachments：获取工作项附件列表](https://help.aliyun.com/zh/yunxiao/developer-reference/listworkitemattachments)
- [ListWorkitemComments：获取工作项评论列表](https://help.aliyun.com/zh/yunxiao/developer-reference/listworkitemcomments)
- [ListWorkitemActivities：获取工作项动态](https://help.aliyun.com/zh/yunxiao/developer-reference/listworkitemactivities)

## 7. 分步提交

1. `docs: plan in-app Bug detail preview`：落档范围、数据边界和验收标准。
2. `feat: aggregate Yunxiao Bug detail data`：增加 DTO、详情/评论/动态适配器、作用域校验和 Mock API 测试。
3. `feat: preview Yunxiao Bug attachments safely`：增加受限附件读取、临时缓存、系统预览与安全测试。
4. `feat: add complete in-app Bug detail view`：增加详情弹窗、内容渲染、缓存、局部错误和浏览器场景测试。
5. `docs: document in-app Bug detail workflow`：更新用户文档并记录验证结果。

## 8. 验收标准

- 点击任意列表项无需打开浏览器即可看到完整只读详情，并能关闭后回到原筛选、页码和滚动位置。
- Markdown、富文本、空描述、长文本和动态自定义字段均可读，危险 HTML 不会执行。
- 附件、评论和动态正常展示；一个附属接口失败不会清空其他已成功内容。
- 图片和常见文本可在应用内预览，其他附件可由系统应用打开；附件下载不泄露 PAT、不接受 HTTP 或任意路径。
- 切换账号或项目后，旧详情及迟到响应不会出现在新作用域；登录失效会回到登录页。
- Rust Mock 覆盖字段映射、项目/Bug 范围、局部失败、授权失败和附件安全；前端覆盖打开/关闭、刷新、缓存、渲染与错误状态。
- `npm run check`、Rust 格式检查、严格 Clippy、工作流测试和项目 Bug 浏览器场景通过。真实云效下的富文本差异、临时附件地址及权限组合保留为人工验收项。

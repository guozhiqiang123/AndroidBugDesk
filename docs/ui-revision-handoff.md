# 首版 UI 修订交接

落实用户 2026-10-09 的登录与两页签要求。基线 `ee6fe17bd5a63769b55a401656925d552348f969`，分支 `agent/codex/gzq-47`。协调员统一维护共享模型、命令注册和 Rust 适配器；前端执行者只修改 renderer 与界面测试。源目录和独立 GZQ-45/48 worktree 未修改。

## 接口与行为

新增 IPC：`check_auth`、`login`、`list_organizations`、`list_projects`、`select_project`、`load_selection_memory`、`remember_selections`、`list_remote_history`。`bootstrap` 增加 user，authorized 表示在线校验通过。旧 set_auth 保留但改为异步校验；clear_auth 异步停止采集。DTO 在 `src/shared/workspace.ts`。Field.showWhenCreate 可选，旧夹具兼容；草稿 Schema 未改。

启动只读本机 bootstrap，显示空令牌输入页；用户点击“确定”后才调用官方 GET /oapi/v1/platform/user，401 为失效；403/网络/格式变化不假称令牌有效。登录成功进入主界面，项目读取失败留在主界面重试。组织、项目、创建字段、成员、标签、迭代及版本来自官方 API；未知必填字段或失效选项阻止写入。历史用只读 POST workitems:search，限定当前项目、Bug、当前账号创建者，50 条一页。创建/上传不确定结果保护沿用原实现。

## 数据兼容

令牌只在用户手动输入并验证成功后接受，仅进程内使用。启动不访问钥匙串，界面移除记住登录。旧 set_auth/login 的 remember 参数保留兼容，标准界面传 false。项目配置以 profile:<userId> 隔离；选择记忆按用户/组织/项目/类型隔离，只保存当前合法下拉值。旧全局示例项目不自动恢复。

新草稿使用独立 draft_scope 绑定。旧未绑定草稿保留但阻止首次提交，需新建当前项目草稿补入需要的信息。已有提交状态沿用固定原项目恢复。不要删旧 SQLite、证据与回执，不用重复创建解决结果不确定。

## 集成与验收

GZQ-46/47 交付为 in_review。GZQ-49 需正常合入本提交与 GZQ-45/48，检查命令注册、共享模型与冲突，重新验证。旧 23 条命令契约要更新新增命令、异步签名及 bootstrap；worktree 不会自动合入。

通过：49 前端测试、24 Rust 测试、TypeScript、Vite、fmt、严格 Clippy、10 场景实际浏览器模拟点击。随后指定令牌的真实只读联调与本地 macOS arm64 包结果见独立交付报告。

未覆盖：真实写入与附件回读、安装后完整 USB 流程、Windows/Linux/Intel Mac、安全存储三平台行为、GitHub Actions、签名公证。UI 评审包未合入 GZQ-45/48，不是首版整合候选包。下一版本复测追加不启动。

## 官方接口依据

- [用户校验](https://help.aliyun.com/zh/yunxiao/developer-reference/getuserbytoken)
- [组织列表](https://help.aliyun.com/zh/yunxiao/developer-reference/listorganizations)
- [项目搜索](https://help.aliyun.com/zh/yunxiao/developer-reference/searchprojects)
- [创建字段配置](https://help.aliyun.com/zh/yunxiao/developer-reference/getworkitemtypefieldconfig)
- [标签列表](https://help.aliyun.com/zh/yunxiao/developer-reference/listlabels)
- [工作项搜索](https://help.aliyun.com/zh/yunxiao/developer-reference/searchworkitems)

真实只读检查已通过用户、组织、完整项目、成员、字段、迭代和当前账号历史读取。角色字段 verifier/participants/trackers 使用 Role 类型，sprint/versions/labels 使用 Application 类型，按官方 ID 映射；Field 增加可选 optionError。标签接口空响应保留为未确认，不假称无标签。分页以已知总页数为先，避免无效末页 x-next-page 导致无限读取。常规测试跳过真实联调测试；只有本次明确授权后，使用 ANDROIDBUGDESK_RUN_AUTHORIZED_READONLY=1 显式运行 ignored 测试，通过 1 项。

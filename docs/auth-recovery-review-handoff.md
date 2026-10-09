首版 GZQ-46 账号绑定恢复返修，基于 UI 最新提交 `91cbfaec0ccb66aa90a46ade472f9c85fe8e08eb`。分支 `agent/codex/gzq-46`。本轮由协调员在独立 worktree 实施，不修改 UI worktree、源目录或其他任务成果。

## 修复与兼容

`submit_bug` 和 `reconcile_submission` 都先通过 `workspace::submission_api` 检查持久的 draft_scope，无论新建、partial、create_unknown 或已经完成。账号、组织或项目不匹配时，在任何云效请求前拒绝。原提交中的组织/项目也必须与当前绑定一致。校验与 API 构建同时持有身份、项目、令牌锁，取得一致快照，避免先验证 Alice 再使用 Bob 的凭据。

Tauri 命令签名、命令注册、DTO、Schema、依赖与锁文件保持。命令委托给 submit_with_state/reconcile_with_state，便于常规测试覆盖完整锁、守卫和提交过程。Submitter 原有组织/项目保护、未知创建不重发、附件核对及不可变 ZIP/独立回执规则保留。

没有 draft_scope 的旧记录仍保留。旧提交恢复返回“历史提交缺少账号绑定，身份未知”，不会静默归属当前账号、读取远端或重建 Bug；旧草稿要求在当前项目新建草稿并核对材料。旧已创建/未知记录不能按这个新草稿操作重新投递；须保留原 ID 和证据，先由人核实原账号与工作项。此次没有添加自动迁移或重新绑定接口。后续若需要可操作的遗留绑定流程，由集成者设计明确的核对步骤和持久审计，不能从同项目或相同标题推定身份。

## 实际验证

- 修复前在本基线复现原独立 Tauri 命令回归：Alice 的 partial 切到 Bob 后返回 submitted，上传 1→3；预期断言失败，exit 101，属于行为失败而非编译失败。
- 持续回归在 tests.rs 增加 7 项：新建/partial/create_unknown 的账号、组织、项目不匹配均零网络请求；补传保留原 ID/ZIP；空候选保持未知且不重建；旧未绑定草稿/partial/unknown 保留；原提交项目与绑定冲突拒绝；身份改变后 API 快照保持原凭据。
- 另通过 Tauri test mock runtime 运行真实 submit_bug/reconcile_submission 边界：原单项回归、三种状态×三类身份不匹配×提交/自动核对/指定 ID 核对、同原账号恢复、旧身份未知拒绝。真实创建/上传/更新和所有 HTTP 请求增量均为零；回到原账号只创建一次并恢复 real-id，ZIP 哈希和旧记录不变。
- 常规检查为 49 前端和 32 Rust 测试；Tauri 边界叠加检查为 35 Rust 测试。真实只读测试默认 ignored，本轮没有启用。TypeScript、Vite、fmt 与严格 Clippy 验证结果随交付记录提供。

为遵守本任务不得修改依赖/锁文件的所有权，附加 Tauri 边界测试保存在交付证据的 `tauri-command-regression.patch`，在最终 tests.rs 上临时应用，执行 `cargo test --manifest-path src-tauri/Cargo.toml --features tauri/test -- --nocapture`，再移除该额外补丁。7 项完整处理路径回归已永久纳入常规 check，不需要 test feature。集成者可下载证据包复跑 Tauri 边界；未来将此 profile 接入自动检查时统一安排 Cargo/CI 文件所有权。

## 交付与剩余门槛

本次修改 commands.rs、workspace.rs、tests.rs 与本交接文档。未改变 renderer、ADB/evidence、workflow、共享模型、Schema、依赖或锁文件。交付补丁与日志哈希在证据 manifest 中，最终 commit 由任务交付记录标识。

保持 in_review 等待技术复核。GZQ-47 的 UI 评审包仍是 `91cbfae`，不包含本返修；用户 UI 实际验收仍待结果。GZQ-49 仍等待技术与 UI 门槛，后续正常合并本分支，再重新验证和生成整合候选。此返修未构建新安装包，没有真实云效/USB、钥匙串访问、远端推送、Actions 或发布；真实媒体与三平台门槛保持。

# GZQ-48 构建配置交接

沿用任务分支 `agent/codex/gzq-48` 和已释放的基线 `ee6fe17bd5a63769b55a401656925d552348f969`。改动限于两份 workflow、`scripts/build-manifest.mjs`、`tests/build-manifest.node.mjs` 与 `docs/ci*.md`。

已实施：四目标安装包的实际文件清单、流式 SHA256、完整 commit 与 package 版本绑定、缺格式失败、固定相对路径、独立候选目录、不覆盖已有结果、artifact 名称含版本/commit/target/run/attempt、安装包与清单一起上传、30 天保留。检查与候选构建沿用既有触发规则，未加入公开发布。配置说明及本地命令见 `docs/ci-builds.md`。

共享文件修改需求：**无**。未更改 package/Cargo 配置与锁文件、Tauri 配置、共享模型、Schema、业务代码或命令注册。自动管理的 `AGENTS.md` 与 `.multica/` 初始变化保留，不纳入本任务提交。

本地脚本测试和 YAML/矩阵检查使用合成文件，不是实际 Tauri 安装包。交付评论中的附件记录本轮检查结果、实际 commit 和哈希。整合者应 cherry-pick 本轮交付 commit，再针对整合后的 commit 运行适用检查；业务代码改变后，旧基线验收不能直接复用。

未运行：GitHub Actions 四目标矩阵、远端 artifact 保留/上传/下载、Windows/Linux/Intel Mac 安装、企业签名、公证、USB 与真实云效流程。本轮没有远端仓库或 Actions/Release 授权，保持本地与 mock 范围；本任务提交技术审查，整体验收保持未完成。

后续用户决策：GitHub 仓库及 Actions 执行授权、目标 Linux 发行版/系统版本、企业分发证书与安装验收范围。现有 macOS arm64/x64、Windows x64、Linux x64 构建配置已保留；签名与系统安装限制不能由本地 YAML 检查消除。

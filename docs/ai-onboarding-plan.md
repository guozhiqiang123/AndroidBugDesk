# “接入 AI”客户端引导计划

状态：已实施并完成本机安装验收
日期：2026-10-22

## 目标

在侧栏新增独立的“接入 AI”页面。发布包内置 Android Bug Desk 的 Codex Skill、MCP 示例和使用说明，页面生成适配当前安装路径的提示语。使用者只需把提示语复制给 Codex，由 Codex 安装 Skill、登记本机 MCP bridge 并完成连接检查。

## 方案

1. Skill 和接入说明使用 Rust `include_str!` 编译进桌面二进制，避免依赖源码仓库或安装目录中的松散文件。
2. 打开“接入 AI”页时，Rust 只把这些固定资源释放到应用本地数据目录的 `ai-integration/codex` 子目录；不接受前端传入任意源路径或目标路径。
3. Rust 返回当前真实桌面可执行文件、Skill 路径、接入包路径和安装提示语。Linux AppImage 优先使用 `APPIMAGE` 指向的持久文件，其他平台使用当前可执行文件。
4. 复制动作由 Rust 写入系统剪贴板，保证 Tauri 在 macOS、Windows 和 Linux 上行为一致。
5. Codex 根据提示语把专用 Skill 安装到自身 Skill 目录，并使用 `codex mcp add android-bug-desk -- <应用路径> --mcp-bridge` 注册服务。客户端不直接修改 Codex 配置，以免覆盖用户已有配置。
6. 页面同时显示登录、项目、AI 接入开关和内置资源状态。最终云效提交仍必须在 Android Bug Desk 的提交预览中由用户确认。

## 安全边界

- 内置资源不包含 PAT、OAuth token、组织 ID、项目 ID、用户信息或本机固定路径。
- 接入命令不提供任意文件导出、任意命令或直接云效提交能力。
- 提示语明确要求 Codex 不读取、复制或输出云效凭据。
- MCP bridge 仍只连接正在运行且已开启“允许本机 AI 接入”的客户端，客户端关闭后能力令牌失效。

## 验收

- 登录后侧栏可进入“接入 AI”，页面不跟新建提交页状态耦合。
- 页面能显示当前系统的真实可执行文件路径和已释放的 Skill 路径。
- “复制给 Codex”写入完整提示语；提示语包含安装、去重更新、连接验证和安全约束。
- 开关可在该页面启停本机 AI 接入，并与设置页保持同一状态。
- Rust 单元测试覆盖资源释放、路径、内容和提示语；浏览器模拟覆盖导航、状态、复制和开关；全量类型、构建、Rust 测试、格式与严格 Clippy 通过。

## 实施记录

- 侧栏新增“接入 AI”，与新建提交、项目 Bug、历史提交和设置并列。
- 接入页提供登录、当前项目、内置接入包三项状态，并复用“允许本机 AI 接入”开关。
- `android-bug-submission` 的 `SKILL.md`、`agents/openai.yaml`、MCP 示例及 README 已编译进桌面二进制；打开页面时释放到应用本地数据目录并生成版本与 SHA-256 清单。
- 生成的提示语包含当前安装程序和 Skill 的真实绝对路径、MCP 去重配置、`workspace_status` 验证步骤、会话重开提示及凭据安全边界。
- 资源释放拒绝符号链接目录和文件；Linux AppImage 使用持久 `APPIMAGE` 路径，其他安装形式使用当前可执行文件。
- 复制由 Rust 系统剪贴板实现，前端不直接修改 Codex 配置，也不接收任意导出路径。

本轮自动化通过 131 项前端测试、219 项 Rust 测试（3 项外部环境测试按设计忽略）、TypeScript、生产构建、Rust 格式、严格 Clippy 和 56 个实际 React/mock IPC 场景。界面截图和结果位于 `artifacts/ai-onboarding-20261022/browser/`。

随后使用固定本机入口生成并校验 macOS ARM64 应用和 DMG，覆盖安装到 `/Applications/Android Bug Desk.app`。原生窗口确认“接入 AI”、真实程序路径、内置 Skill 状态和复制成功提示；释放文件与源码内置 Skill 哈希一致，安装文件与构建产物一致，启动后没有新增崩溃报告。产物和安装回执位于 `artifacts/local-ai-onboarding-macos-arm64-20261022/`，DMG SHA-256 为 `6aecc4a03c650c9735ea1d57207acfc2453ae0ae4a711ddd7c00b72534f4929f`。本轮未连接真实 USB，也未写入真实云效。

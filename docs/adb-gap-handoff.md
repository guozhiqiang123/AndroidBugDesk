# GZQ-45 采集基础能力交接

本次基于 `ee6fe17bd5a63769b55a401656925d552348f969`，任务分支 `agent/codex/gzq-45`。仅补齐 Rust 库接口及其内联测试；当前界面和 23 个 Tauri 命令仍沿用基线。未改变共享 model、草稿/Manifest Schema、SQLite、依赖或命令注册。新增类型是内部采集结果，不是草稿字段。

## 已实施

### 按需系统报告

`adb::bugreport(adb_path, serial, store_root, timeout_seconds)` 返回 `Result<BugreportCapture>`。必须显式指定 USB serial；拒绝空值、选项前缀和命令注入字符。只执行参数数组 `adb -s SERIAL bugreport DIRECTORY`，不使用 shell、不清空日志、不请求 root、不回退其他设备。超时限制 1–600 秒。

输出使用 Rust 管理根目录下独立的 `bugreports/<UUID>/`；目录名称由后端生成，不接受输出路径参数。目录 canonical containment 检查阻止既有符号链接将输出转移到根目录外；附件导入也使用同一检查。该检查不是针对恶意本地进程并发替换目录的 OS 沙箱。

`stdout.log`、`stderr.log` 和 `receipt.json` 与原始报告分开保存。回执包含 serial、实际开始/结束时间、超时标志、进程退出码（信号退出可能为空）、failure 和可选 attachment。进程启动失败、非零退出、超时、无 ZIP、多 ZIP、符号链接 ZIP 或损坏 ZIP 都不返回成功附件；保留部分文件用于诊断。超时停止并回收本接口启动的确切子进程。合法 ZIP 经现有 `inspect` 复制为带 SHA256 的未确认附件，角色 `report`、来源明确为未脱敏。ZIP 检查是容器可读性检查，不保证内部系统报告内容完整，也不解压报告。

参数/目录等前置错误返回 `Err`；已执行的采集失败返回带 failure 的回执。调用者必须先检查 failure，不能把有回执或有文件视作成功。取消 future 会触发既有 `kill_on_drop`；强制退出或取消前的任务状态持久化仍需集成者处理，不能凭截止时间推定报告已完成。

### 前 120 秒 / 后 30 秒日志窗口

`adb::ProblemWindow::new(marked_at)` 固定 `due_at = marked_at + 30 秒`，可序列化为待固定任务。`wait_ready(cancel_receiver)` 在前台等待截止时间，取消或发送端关闭返回 `Interrupted`；截止时间被修改则拒绝。`Ready` 仅表示电脑截止时间已到，不能证明实际日志齐全。

`evidence::summarize_log_window(root, pinned_logs, window, observed_until, device_minus_host_micros, clock_note, gaps)` 从不可变的日志附件提取窗口，返回窗口附件、`LogWindowSummary` 和独立 `summary.json` 路径。每个输入前后核对 SHA256；输出使用新目录，不改写 raw、既有证据目录或 ZIP。所有 PID 的记录都会保留；没有按目标进程过滤。

- 输入必须是 `inspect` 固定过的 `log` / `text/plain` 附件，按原始采集顺序传入，不能直接传仍在追加的 collector 文件。读取逐行完成，单条记录上限 1 MiB。
- 使用 `logcat -v epoch` 时间；设备时间减电脑时间的微秒差由调用者提供，并必须说明依据。没有对应依据时传 `None`：摘要记录时钟未采集、空覆盖、零选中记录，保留原始附件供人工核对，不假设两台时钟一致。
- requestedStart/requestedEnd 是目标窗口；coverageStart/coverageEnd 仅取实际选中记录的边界，不使用会话起点或固定动作时间代替。不会选择超过 observedUntil 的记录。窗口可缺前段、后段或两段。
- `LogGap` 使用电脑时间、可为空的开始/结束和原因。只排除确定在窗口外的缺口；未知边界保守保留。旧 session 字符串 gaps 需由集成者转为未知边界的 `LogGap`，不能丢弃。
- 前/后范围不足、观察过早、未知时钟、未解析行、末尾不完整记录、时间倒退/分段重叠分别记录；断线缺口即使首尾时间足够也让 rangeCovered 为 false。
- rangeCovered 只表示实际时间边界达到请求范围且没有已知缺口/限制，不能证明设备缓冲区无丢失或每秒有日志。空日志不会被认定已覆盖。分段切开记录时采取保守不足标记，当前没有跨文件重组或厂商多行日志解析。
- summary.json 保存每个来源附件的 id、SHA256、大小、来源说明。单来源时窗口附件 derivedFrom 指向该附件；多来源不编造单一 parent，完整来源关系保存在摘要。

### 精确文本替换派生副本

`evidence::redact_text(root, original_attachment, rules)` 接受不超过 32 MB 的完整 UTF-8 日志与 1–100 条 `TextRedactionRule { label, literal }`。只按大小写敏感的精确文本替换为 `[REDACTED]`；literal 不作为正则执行，重叠时最长文本优先，替换结果不递归参与替换。规则必须有非敏感标签，literal 非空且不超过 4096 字节。

返回新 attachment、receipt 和 receipt_path。新附件默认未确认，derivedFrom 指向原附件，保留原覆盖时间；回执存原 SHA256、派生 SHA256、实际方法、标签与匹配次数，`completeRedaction=false`。不将敏感 literal 写入回执。原文件保持不变且哈希检查通过；方法未匹配的内容仍然存在。该能力不自动检测个人信息、不处理图片/视频/ZIP，也不宣称完整脱敏。

## GZQ-49 集成请求

1. 在 Rust 命令中从 `AppState` 读取 adb_path、固定 session serial 和 store.root，不让 renderer 传任意路径、命令或系统报告输出目录。调用前确认草稿可编辑和对应会话；采集错误记录来源并允许人工补充。系统报告须由用户明确按需触发，附件仍需预览确认。
2. 标记时先保存 ProblemWindow、draft/session 身份和待固定状态，再等待。停止采集、断线、退出或任务取消均需保存 observedUntil 和缺口。重启将未完成任务恢复为中断，不能因当前时间超过 dueAt 宣称后 30 秒已采集；新数据应记录为事后补采。
3. 固定 collector 输出为独立 raw 附件后生成摘要，获取实际设备/电脑时钟估计并明确精度，转入已有和新增采集缺口。避免 collector 对 session 的写入覆盖调度状态。初始 buffer 时间只能来自解析结果。
4. 摘要 JSON、脱敏回执需经现有受限文件入口导入为证据附件或后端管理的补充元数据，随最终证据清单交付；不能只交 window.log 而丢失缺口、时间依据及多来源关系。需要扩展草稿/Manifest 时由集成者更新 domain、Schema、fixtures 并验证两端。
5. 脱敏命令仅接收草稿/附件 id 和规则，由 Rust 查找已导入原文件。显示替换次数和适用范围，要求用户复核并选择提交哪些原始/派生附件；保留本地原件不能等同于默认将敏感原件一并上传。证据已经固定后应创建新草稿，不改变原 ZIP。

## 本地与 mock 验证

内联新增 16 项 Rust 测试：两种显式 serial 和包含空格/分号的根路径；超时停止确切子进程及部分文件保留；启动/非零退出/无输出/损坏/多 ZIP/符号链接失败；参数注入与超时范围；延迟截止/取消/发送端关闭；准确窗口边界、PID 变化、时钟差、多来源、前/后不足、断线与未知缺口、未知时钟、格式/截断/时间倒退；raw 哈希不变；脱敏来源与稳定哈希、精确匹配/最长匹配、次数、无完整脱敏声明、无效文本/规则和内容变更拒绝；各输出目录符号链接逃逸拒绝。

fake ADB 为测试临时可执行文件，未调用本机真实 ADB 或 USB 设备。执行环境为本机 macOS arm64；Unix fake 进程/符号链接测试有平台条件，不能据此宣称 Windows/Linux 已验证。

最终验证结果和交付 commit 由 GZQ-45 的最终交付评论记录，完整日志位于本任务工作目录的相邻 `verification/gzq-45/`（不进入源控）。初轮超时测试使用 200 ms 导致 fake 进程来不及产生部分文件；调整为 1 秒后重新验证。生产接口超时策略未因该测试调整而变化。

## 未验证与未接入

本轮没有新真机采集、真实云效写入、PAT 读取、远端 push、Actions、桌面打包或安装验收。bugreport 厂商行为、大型报告/磁盘不足、真实多设备/断线/进程重启、时钟跳变、权限拒绝和安装版退出恢复需后续实测。命令/UI、自动调度持久化、摘要/脱敏回执入最终清单仍待 GZQ-49。构建 fingerprint、AAB/安装 APK 身份和 mapping 不在本轮新增采集范围，未知状态保持不变。基础能力交付不能代表完整提交流程或首版整体验收通过。

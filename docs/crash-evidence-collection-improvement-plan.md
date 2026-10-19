# Android 崩溃证据采集能力改造方案

确认日期：2026-10-16。

本文用于交接 AndroidBugDesk 的崩溃证据采集改造。结论是：当前客户端对 Android 10 及部分旧设备的事后日志能力不足，尤其无法稳定保留已经退出进程的 Native 崩溃堆栈。问题不在设备没有产生日志，而在兼容命令和过滤模型会把有效证据丢掉。本方案只描述 AndroidBugDesk 客户端改造，不处理业务应用或广告 SDK 本身的崩溃修复。

关联缺陷：[BNYF-162](https://devops.aliyun.com/projex/bug/BNYF-162)。

## 实施批次与当前状态

本改造按可以独立验收、独立回退的三个批次实施，避免同时改动设备命令、隐私过滤和实时会话后难以判断回归来源：

| 批次 | 范围                                                                                  | 当前状态                     |
| ---- | ------------------------------------------------------------------------------------- | ---------------------------- |
| A    | API 能力探测、Crash buffer 完整记录过滤、DropBox 定向过滤、`ApplicationExitInfo` 分流 | 已实施，等待真机验收         |
| B    | 系统事件白名单、历史 PID 汇总、回执来源与过滤结果展示                                 | 已实施，等待真机验收         |
| C    | 恢复实时复现入口，并为旧系统补齐进程重启和 Crash 记录兼容                             | 已实施，等待跨平台与真机验收 |

批次 A、B 的实现约束：Crash/DropBox/系统事件原始整机输出只存在于单次命令返回的受限内存中；附件仅写入已归属到目标包的完整记录；解析器、命令编排和结果展示分层。批次 C 在 `--uid` 不可用时将普通日志流与 Crash 流分开：普通流按每两秒刷新一次的目标 PID 白名单过滤，Crash 流按完整记录过滤；过滤发生在写盘之前。界面提供开始、标记和停止，标记后继续采集 30 秒并自动结束，提前停止会记录覆盖缺口。

## 一、现场结论

现场设备及应用信息：

- 设备：Samsung SM-G9730；
- 系统：Android 10，API 29；
- 应用：`com.ludashi.superboost`；
- 版本：4.2.5，versionCode 101。

原缺陷附件是崩溃后的事后补采，只包含当时仍在运行的主进程信息，没有保存真正的 Fatal 堆栈。在同一设备、同一安装包的后续现场取证中，于 2026-10-16 19:23 捕获到真实崩溃：

- 类型：主进程 `SIGABRT`；
- 崩溃进程 PID：7777；
- 崩溃线程：`DefaultDispatcher`；
- ART abort message：`Check failed: self == thread_running_gc_ Only GC-running thread should access the mark stack in the GC exclusive mark stack mode`；
- 关键调用：`com.advertising.source.admob.GetAdmobPriceUtilKt.findAdMobAdPrices`；
- 上层调用：`AdMobLoader$getAdValue$2.invokeSuspend`。

崩溃后应用已经重启，当前主进程 PID 变为 5601。未过滤读取设备 Crash buffer 时，完整 Native 堆栈仍然存在；按当前进程 PID 过滤后，该堆栈会消失。此外，Native tombstone 的多数行由 `debuggerd`/`tombstoned` 进程输出，日志行 PID 并不等于业务应用 PID。由此可以确认：按当前 PID 逐行过滤不是 Native 崩溃的有效归属方式。

现场还确认了两项 Android 10 兼容问题：

```text
adb logcat -d -v epoch --uid=<uid> -b crash
=> Unrecognized Option

adb shell dumpsys activity exit-info <package>
=> Unknown command / Bad activity command
```

而以下不带 UID 过滤的命令可以读取到完整 Native 崩溃记录：

```text
adb logcat -d -v threadtime -b crash
```

设备 DropBox 中也保留了对应 `SYSTEM_TOMBSTONE`。这说明客户端应当读取候选记录后在内存中按“完整崩溃记录”筛选，而不是完全依赖设备端 UID/PID 过滤。

## 二、当前实现缺口

### 2.1 Crash buffer 的旧系统回退会丢失崩溃进程

当前 `src-tauri/src/post_capture.rs` 优先使用 `logcat --uid`。设备不支持时，会查询当前 `pidof`/`ps`，再使用 `--pid` 或在本地逐行保留当前 PID。这个策略存在两层问题：

1. 崩溃进程已经退出并重启，当前 PID 与崩溃 PID 不同；
2. Native 堆栈由 `debuggerd` 等系统进程代为输出，即使已知旧 PID，逐行 PID 过滤仍会删除大部分堆栈。

现有回执虽然正确标记了 `historicalCoverageLimited=true`，但附件本身已经没有可用于定位的核心证据。

### 2.2 退出原因命令没有按 Android API 分流

当前所有设备都执行 `dumpsys activity exit-info`。`ApplicationExitInfo` 是 Android 11/API 30 起才具备的能力，Android 10 返回命令不支持。现在只会得到一个“未获取到”的结果，没有继续采集 DropBox、Crash buffer 或系统进程事件作为兼容证据。

### 2.3 普通日志只按应用 UID 过滤会漏掉系统侧事件

ActivityManager、LMKD、WindowManager、`am_crash`、`am_anr`、`am_proc_died` 等记录由系统 UID 输出，不属于目标应用 UID。只读取应用 UID 日志无法覆盖应用退出、低内存杀进程和部分 ANR 上下文。

### 2.4 实时复现采集后端存在，但当前界面不可用

`src-tauri/src/adb.rs` 和 `src-tauri/src/commands.rs` 已有 `start_session`、`mark_problem`、`stop_session`、日志轮转、采集间隙和“标记后继续采集 30 秒”等基础能力，但当前前端没有调用这些命令。现有设计文档也明确限定为只做事后补采。因此测试人员只能在问题发生后开始读取有限的环形缓冲区。

旧实时采集后端本身也依赖 `logcat --uid`，恢复入口时必须同时补齐 Android 10 的兼容策略，不能只恢复按钮。

### 2.5 现有自动化测试固化了错误回退模型

`src-tauri/src/post_capture_tests.rs` 当前断言 Android 10 风格设备在 `--uid` 不支持时退回当前包 PID，并验证逐行过滤不保存其他应用日志。这些测试保护了隐私边界，但也固化了会丢失历史 Native 崩溃的实现，需要用“完整记录归属过滤”测试替换。

## 三、改造目标与边界

### 3.1 必须达到的目标

- Android 10/API 29 及以上设备均能采集 Java 崩溃、Native 崩溃、ANR 和进程异常退出的有效证据；
- 应用崩溃并重启后，仍能从现存系统缓冲区中找到旧进程记录；
- Native 堆栈由系统进程输出时，仍能保留完整记录；
- 不把整机原始 logcat 保存到磁盘或作为附件上传；
- 回执准确说明实际命令、Android API、过滤模式、覆盖限制、兼容回退和失败原因；
- 保留现有超时、体积上限、单项失败隔离、指定设备和项目包名门禁。

### 3.2 不承诺的能力

- 不使用 root 权限直接读取 `/data/tombstones`；
- 不读取目标应用私有目录或数据库；
- 不清空 logcat、不重启目标应用、不修改设备设置；
- 日志已被环形缓冲区覆盖、设备已断开或厂商禁止读取时，不承诺恢复历史证据；
- 项目自定义的 Java CrashHandler 文件可作为以后扩展，但不能作为 Native 崩溃的核心路径。

## 四、目标采集架构

### 4.1 先识别设备能力

每个任务开始时读取并记录：

```text
adb shell getprop ro.build.version.sdk
```

根据 API 和命令探测结果选择策略，不要仅根据机型或异常文本猜测能力。能力探测结果写入回执，同一任务内复用。

| 能力           | API 30 及以上                          | API 29 及以下                                         |
| -------------- | -------------------------------------- | ----------------------------------------------------- |
| 应用退出记录   | `dumpsys activity exit-info <package>` | 明确标记不支持，改采 Crash buffer、DropBox 和系统事件 |
| `logcat --uid` | 支持时用于应用自身普通日志             | 支持则使用；不支持时启用本地过滤                      |
| Native 崩溃    | 完整 Crash record 过滤                 | 完整 Crash record 过滤                                |
| 系统侧进程事件 | 本地按目标包/UID/历史 PID/事件标签过滤 | 同左                                                  |

API 版本只是首选分流条件；命令仍需检查退出码和输出，因为厂商实现可能不同。

### 4.2 Crash buffer 必须按完整记录过滤

崩溃专项始终允许读取未过滤的 Crash buffer，但原始输出只能存在于进程内存或自动删除的受控临时流中：

```text
adb logcat -d -v threadtime -b crash
```

解析器必须先切分完整候选记录，再判断该记录是否属于目标应用。不能先按日志行 PID 删除内容。

可用于识别记录归属的锚点包括：

- Native：`>>> <package> <<<`、`Cmdline: <package>`、`pid: ..., name: ..., >>> <package> <<<`；
- Java：`FATAL EXCEPTION` 后的 `Process: <package>, PID: ...`；
- 系统记录：明确包含目标包名、目标 UID、已识别的历史 PID 或进程名 `<package>:<suffix>`。

记录切分需保留：

- Native 的 `*** *** ***` 头、Build fingerprint、ABI、Timestamp、pid/tid/name、signal、abort message、寄存器、backtrace 和后续 continuation 行；
- Java 的异常线程、Process 行、完整 cause/suppressed 链和线程栈；
- 同一崩溃记录内由 `debuggerd`、`tombstoned` 等不同 PID 输出的行。

推荐处理顺序：

1. 流式读取整个 Crash buffer，施加命令超时和最大内存限制；
2. 根据 Native/Java 记录边界切分候选记录；
3. 候选记录收束后，用包名、进程名和 UID 等判断归属；
4. 只将命中的完整记录写入附件；
5. 丢弃原始整机输出，不落盘、不写诊断日志；
6. 回执记录 `filterMode=localCrashRecord`、匹配记录数和 `rawDeviceLogPersisted=false`。

若没有目标应用记录，结果应为“未获取到：Crash buffer 中没有目标应用匹配记录”，不得保存其他应用日志充数。

### 4.3 增加 DropBox 定向补采

Crash/ANR 专项增加系统 DropBox 候选来源，至少探测：

- `SYSTEM_TOMBSTONE`；
- `data_app_crash`；
- `data_app_native_crash`，仅在设备存在该 tag 时；
- `data_app_anr`。

DropBox 输出也必须先按完整条目切分，再按目标包、UID或进程名筛选，只保存命中条目。禁止把完整 `dumpsys dropbox` 输出作为默认附件。DropBox 不可访问、tag 不存在或无匹配条目时，写入明确的“不可用/未命中”回执，不影响其他来源继续采集。

Crash buffer 与 DropBox 若内容重复，可按时间、进程、signal/异常摘要及正文哈希去重；无法可靠去重时允许分别保存并注明来源，优先保证证据完整。

### 4.4 增加系统事件上下文

进程退出和 ANR 诊断需要读取有限的系统日志候选，再在内存中按目标应用筛选。至少覆盖：

- `am_crash`；
- `am_anr`；
- `am_proc_died`；
- `am_kill`；
- `am_low_memory`；
- LMKD/lowmemorykiller 中明确指向目标包、UID 或历史 PID 的记录；
- ActivityManager 中明确指向目标包的进程启动、结束和异常记录。

不要把“当前 PID”当作唯一历史身份。可从 Crash record、DropBox、`ps`、`pidof`、`dumpsys package` 和事件记录中汇总候选 PID，并同时使用包名、进程名和 UID 校验。保存附件前仍需确保不存在无关应用的完整日志内容。

### 4.5 API 感知的退出原因策略

- API 30 及以上：保留 `ApplicationExitInfo`，并记录命令输出是否包含目标退出记录；
- API 29 及以下：不再执行已知不支持的 `exit-info` 命令；回执写明“系统版本不支持 ApplicationExitInfo”，并列出已执行的替代来源；
- API 30 及以上若命令被厂商禁用或执行失败：同样启动 Crash buffer、DropBox 和系统事件回退，保留原始失败原因。

回执中应区分“系统不支持”“命令失败”“没有匹配记录”“日志可能已覆盖”，不能统一显示为采集失败。

### 4.6 恢复可操作的实时复现采集

在事后补采之外，恢复一套独立且清晰的流程：

1. 开始复现采集；
2. 用户在手机上复现；
3. 标记问题；
4. 后端继续采集 30 秒；
5. 结束采集并生成附件与回执。

优先复用现有 `start_session`、`mark_problem`、`stop_session`、轮转、日志间隙和大小控制能力。前端必须显示当前设备、目标包、采集状态、已运行时长、是否出现日志间隙以及停止入口。

实时采集同样需要 Android 10 回退：

- 应用自身日志可在 `--uid` 不支持时使用本地包/UID/进程识别过滤；
- Crash buffer 和系统异常记录使用完整记录过滤，不能逐行只看当前 PID；
- 设备断线、进程重启或 PID 变化不能静默切换设备，也不能把会话永久留在进行中；
- 保留现有 5 MB 轮转和总量上限，最终附件只包含目标相关内容。

事后补采继续保留，作为未提前开始会话时的兜底入口。两种模式在界面和回执中必须明确区分，不能把事后现存日志描述为完整复现窗口。

## 五、建议修改范围

| 文件/模块                             | 主要工作                                                                  |
| ------------------------------------- | ------------------------------------------------------------------------- |
| `src-tauri/src/post_capture.rs`       | API 探测、Crash record 解析、DropBox/系统事件采集、退出原因分流、回执字段 |
| `src-tauri/src/adb.rs`                | 抽取可复用的记录解析和过滤能力；修复实时会话在旧系统上的日志回退          |
| `src-tauri/src/commands.rs`           | 复核实时会话命令生命周期、错误回收和前端调用契约                          |
| `src-tauri/src/post_capture_tests.rs` | 用 Android 10/Native 崩溃 fixture 替换当前“只过滤当前 PID”的断言          |
| `src/shared/post-capture.ts`          | 调整崩溃/ANR 推荐项、结果状态和兼容回退元数据                             |
| `src/renderer/post-capture.tsx`       | 展示来源、覆盖限制、兼容回退、未命中原因                                  |
| `src/renderer/main.tsx`               | 接入“开始复现采集/标记问题/结束采集”流程                                  |
| `docs/post-capture.md`                | 实施完成后同步新的能力边界和隐私模型                                      |

解析和过滤逻辑应形成独立、可单元测试的纯函数或小模块，避免继续把命令执行、文本解析、隐私过滤和附件写入全部堆在一个流程中。

## 六、分步实施与提交建议

### 第一步：P0，修复 Android 10 事后崩溃采集

- 增加 API/命令能力探测；
- Crash buffer 改为完整记录过滤；
- 增加 DropBox 定向补采；
- `ApplicationExitInfo` 按 API 分流；
- 更新回执字段和结果提示；
- 加入 Samsung Android 10 的 Java/Native fixture 测试。

这一步完成后，即使不恢复实时入口，也必须能解决 BNYF-162 暴露的“设备有堆栈但附件没有堆栈”问题。

建议提交信息：`修复旧系统崩溃证据补采`。

### 第二步：P1，补齐系统退出与 ANR 上下文

- 增加系统事件白名单和本地目标过滤；
- 汇总历史 PID、UID、包名和进程名；
- 完善 Crash/ANR 推荐组合及状态文案；
- 增加跨应用隐私隔离测试。

建议提交信息：`补齐应用退出和ANR系统证据`。

### 第三步：P1，恢复实时复现采集入口

- 接通现有会话后端；
- 修复旧系统 UID 不支持时的实时过滤；
- 增加开始、标记、结束、断线和超时 UI；
- 保持轮转、总量上限和日志间隙回执。

建议提交信息：`恢复问题复现日志采集流程`。

### 第四步：文档与跨设备验收

- 同步 `docs/post-capture.md` 和用户说明；
- 固化 Android 10、Android 11+ 及至少两个不同厂商的 fixture；
- 输出每类场景的示例回执和验证报告。

建议提交信息：`完善崩溃采集文档与兼容验证`。

## 七、自动化测试要求

至少新增以下测试，不能只验证命令字符串：

1. Android 10 不支持 `--uid`，未过滤 Crash buffer 中存在目标 Native 崩溃，能够保存完整 abort message 和 backtrace；
2. 崩溃 PID 与当前 PID 不同，仍可命中历史崩溃；
3. Native 记录由 `debuggerd` PID 输出，仍保留记录内所有 continuation 行；
4. Crash buffer 同时包含目标应用和其他应用，附件只包含目标应用记录；
5. Java `FATAL EXCEPTION` 保留完整 cause/suppressed 链；
6. Android 10 不执行 `exit-info`，回执写明系统不支持及替代来源；
7. Android 11 及以上继续采集 `ApplicationExitInfo`；
8. DropBox 有目标 tombstone 时保存命中条目，没有命中时不保存整机输出；
9. 原始未过滤输出不会写入附件目录、应用日志或失败诊断目录；
10. 命令超时、输出超限、设备断线和部分来源失败时，其他来源仍能生成结果；
11. 实时采集期间目标进程重启，采集不中断且不会切换到其他设备；
12. 记录解析器面对 Samsung、Pixel 及至少一个国产厂商格式时均能正确切分。

fixture 应脱敏后进入测试资源，保留真实的多 PID、跨行堆栈和混入其他应用记录等结构特征。不要只用过度简化的单行假日志。

## 八、真机验收标准

### 8.1 BNYF-162 同设备回归

在 Samsung SM-G9730、Android 10 上安装待验版本，触发同类 AdMob Native 崩溃。应用自动重启后再执行“补采崩溃”，附件必须包含：

- `SIGABRT`；
- ART abort message；
- 崩溃进程旧 PID，而不是只出现重启后的当前 PID；
- `findAdMobAdPrices`；
- `AdMobLoader$getAdValue$2.invokeSuspend`；
- 完整 Native backtrace 的上下文。

回执必须明确显示 Android 10 不支持 `ApplicationExitInfo`，并列出实际使用的 Crash buffer/DropBox/系统事件来源。

### 8.2 隐私和串包验证

在采集前人为制造另一个应用崩溃。最终附件、回执和客户端自身日志中不得出现另一应用的完整堆栈、包名上下文或原始整机 Crash buffer。允许回执记录“已丢弃非目标记录数量”，但不得包含其正文。

### 8.3 Java 崩溃、ANR 和进程退出

- Java 崩溃后重启：附件包含 `FATAL EXCEPTION` 和完整 cause 链；
- ANR：附件包含目标进程 ANR 记录、系统事件和可用的 DropBox 条目；
- 低内存/系统杀进程：附件包含明确指向目标包或 UID 的系统事件；
- 无异常记录：显示“未获取到匹配记录”，不生成无关日志附件。

### 8.4 Android 11 及以上回归

在支持 `ApplicationExitInfo` 的设备上验证原能力没有退化；退出记录、Crash record 和回执时间范围一致，且不会因为新增兼容逻辑重复上传大量相同内容。

### 8.5 实时复现流程

- 能开始、标记和结束会话；
- 标记后继续采集 30 秒；
- 崩溃导致进程重启后仍保留崩溃前后日志；
- 断线、取消和客户端退出能正确终止子进程并落回执；
- 日志轮转、总量上限和间隙提示有效。

## 九、隐私与安全红线

- 未过滤整机 logcat/DropBox 内容只能短暂存在于内存或自动删除的受控临时流；
- 禁止将完整原始缓冲区写入附件、应用运行日志、错误提示或诊断残留目录；
- 必须先完成目标记录过滤，再调用现有附件保存流程；
- 解析失败时宁可返回“无法安全过滤”，也不能回退上传整机日志；
- `adb bugreport` 仍属于用户明确选择的整机诊断兜底，不应因 Crash buffer 解析失败而静默自动执行；
- 任何新命令继续绑定用户选定的设备序列号和项目配置包名。

## 十、开发完成定义

以下条件全部满足才视为本改造完成：

- 第七节自动化测试通过；
- 第八节真机验收至少覆盖 Samsung Android 10 和一台 Android 11+ 设备；
- BNYF-162 同类 Native 崩溃在进程重启后可以从客户端附件直接定位；
- 未发现其他应用日志泄漏；
- 前端构建、Rust 测试、格式检查和生产构建通过；
- `docs/post-capture.md` 已同步最终实现，不再描述与代码冲突的“只按 UID/PID 采集”边界；
- 每个阶段独立提交，提交中不混入业务应用或广告 SDK 的修复。

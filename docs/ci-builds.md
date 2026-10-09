# 桌面候选构建与清单

当前技术选择是 Rust + Tauri 2 和 React/TypeScript。此文档说明已经实施的 CI 配置及本地模拟验证，不代表 Actions 已运行、安装包已生成或跨平台验收通过。

## 检查、候选构建与公开发布

- `Validate`（`.github/workflows/check.yml`）：PR 和 main/master push 运行 TypeScript、前端测试、合成安装包清单测试、Schema/夹具一致性、前端构建、Rust 测试、rustfmt 和严格 Clippy。
- `Desktop bundles`（`.github/workflows/bundle.yml`）：只在手工 dispatch 或 `v*` tag push 时构建候选包。每个目标独立执行；一个目标失败不会取消其他目标，但该次矩阵不能作为四目标通过证据。
- 公开 Release：当前没有发布 workflow，也没有 Release 写权限或自动发布步骤。普通分支 push 只检查。候选 artifact 上传不构成用户批准分发。

所有 workflow 使用 `contents: read`，前端依赖用 `npm ci`，Rust 测试、Clippy 和 Tauri 打包使用 `--locked`。现有 `package-lock.json` 和 `src-tauri/Cargo.lock` 由集成者维护，本任务未更改。Rust stable 和 runner 标签沿用基线；锁文件不锁编译器或 runner 镜像版本，实际 Actions 验收仍需保留对应 run 的工具链日志。

## 原生矩阵与预期文件

| 构建系统       | Rust target              | OS / arch     | 必须存在的安装产物（相对 `bundle/`） |
| -------------- | ------------------------ | ------------- | ------------------------------------ |
| macos-latest   | aarch64-apple-darwin     | macos / arm64 | 根目录 `*.app.zip`，`dmg/*.dmg`      |
| macos-latest   | x86_64-apple-darwin      | macos / x64   | 根目录 `*.app.zip`，`dmg/*.dmg`      |
| windows-latest | x86_64-pc-windows-msvc   | windows / x64 | `nsis/*.exe`                         |
| ubuntu-22.04   | x86_64-unknown-linux-gnu | linux / x64   | `deb/*.deb`，`appimage/*.AppImage`   |

每行的 bundle 根目录是 `src-tauri/target/<target>/release/bundle/`。macOS 的 `.app` 先通过 `ditto --keepParent --sequesterRsrc` 归档，随后对实际 ZIP 文件计算哈希；不对应用目录伪造文件哈希。清单只读取表中目录的直接文件，不递归采集应用内部文件、构建助手、MSI、RPM 或其他目标格式。

两个 macOS CPU 架构及 Windows/Linux x64 已由当前任务与仓库说明确定。Ubuntu 22.04 是当前构建基线；用户部署的 Linux 发行版、版本与桌面环境仍未确认。构建 runner 标签不能证明最低系统支持范围。

## 清单生成与上传

在仓库根目录运行：

```sh
node scripts/build-manifest.mjs --target aarch64-apple-darwin
```

可用 `--bundle-dir <path>`、`--output-dir <path>` 指定输入及输出，支持空格和中文路径。默认输出目录是 `artifacts/candidates/`；输出与 bundle 目录不能相互包含。

生成的目录名同时也是 Actions artifact 名称：

```text
AndroidBugDesk-<package版本>-<完整40位commit>-<Rust target>-run<run ID>-attempt<attempt>
```

本地执行没有 GitHub run 信息时省略 run/attempt 后缀。安装文件保留 Tauri 原始名称与相对目录，避免改变安装格式。一个目录中保存：

- `build-manifest.json`：schemaVersion 1、package 名称与版本、Git HEAD 实际 commit、是否存在已跟踪修改、OS/arch/target、实际 Node hostPlatform、生成时间、artifact 名称及可取得的 CI run/attempt。
- 每个安装文件：相对 artifact 根目录的 `/` 分隔路径、格式、真实字节数与小写 SHA256。
- `SHA256SUMS`：每个安装文件及 `build-manifest.json` 的 SHA256，供下载后检查。

脚本不接受 `--commit` 占位输入。HEAD 不存在、项目不在 Git 仓库根目录、版本无效、`GITHUB_SHA` 与 HEAD 不同、CI 已跟踪文件有修改、CI run 信息不完整、任何预期格式缺失、空文件、隐藏文件、伪装成安装文件的目录或符号链接均失败。其他格式的文件不会满足缺失项。每种预期格式可以有多个文件，全部纳入清单。

脚本先复制允许的文件，再对准备上传的副本逐个流式计算哈希；原 bundle 不修改。失败时清理本次临时输出，不留下可上传的半份候选目录；同名候选目录已存在时失败，不覆盖旧结果。本地有已跟踪修改时记录 `trackedChanges: true`；该输出只能用于本地检查。CI 必须是与 `GITHUB_SHA` 一致的干净 checkout。

清单步骤成功后通过 `GITHUB_OUTPUT` 提供 artifact 名称和目录。上传步骤只上传这个目录，`if-no-files-found: error`；保留期显式设为 **30 天**，并在名称中区分同 commit 的不同 run/attempt。包与清单一起上传，不能只上传清单来掩盖缺包。保留期仍受仓库/组织设置及删除策略约束，长期验收证据需要另行保存。[upload-artifact 官方说明](https://github.com/actions/upload-artifact#retention-period)

下载并解压 artifact 后，以解压目录为根目录，在提供 GNU `sha256sum` 的系统运行 `sha256sum --check SHA256SUMS`。macOS 可用 `shasum -a 256 -c SHA256SUMS`。Windows 可用 `Get-FileHash -Algorithm SHA256` 与清单逐项比对。SHA256 检查的是文件一致性，不能替代签名或安装验收。AppImage 下载后可能需要 `chmod u+x <文件>`，Actions artifact 不保留原文件执行位。[upload-artifact 权限说明](https://github.com/actions/upload-artifact#permission-loss)

## 本地检查与验收边界

```sh
node --test tests/build-manifest.node.mjs
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

清单测试为 Node 原生测试，独立于现有 Vitest，用临时 Git 仓库和合成小文件验证四目标路径、真实字节哈希、空格/中文路径、多文件、缺任一格式、目标格式过滤、未知 commit、CI commit 冲突、空文件、输出不覆盖、CLI 参数和 Actions 输出。合成 `.dmg`/`.exe` 等文件只是测试字节，不是可安装包；符号链接场景在 macOS/Linux 测试，Windows 无需提升权限来建立测试符号链接。

当前仅实施与本地模拟验证。四目标的 Actions 编译、artifact 上传/下载、安装及 USB/凭据/媒体流程均未在本任务实际运行。源基线已有的 macOS arm64 `.app` 证据见 `docs/acceptance.md`，不作为本次新配置的矩阵证据。

macOS 当前只配置 `APPLE_SIGNING_IDENTITY: "-"` 的临时签名，未配置企业 Developer ID、公证及 stapling；Windows 未配置企业代码签名；Linux 未配置签名分发渠道。Gatekeeper、SmartScreen、Linux WebKit/DBus/FUSE/USB 权限及目标机安装限制需要对应系统实测。没有 GitHub 仓库与运行授权前，不推送、不 dispatch、不上传、不创建 Release。

阶段 5 应取得同一交付 commit 的四个成功 job、run ID、artifact ID/名称、下载后的完整清单及哈希；阶段 6 再逐系统记录实际安装结果。一个 job 的成功、仅有 YAML 配置或前端 build 均不能宣布三平台桌面打包或整体验收通过。

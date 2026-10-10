# Android Bug Desk

Rust + Tauri 2 + React/TypeScript 的 Android USB 缺陷采集桌面客户端。源码独立于 Android 业务仓库。

2026-10-09 按用户要求将实施计划中的 Electron 方案改为 Tauri，目标为 macOS Apple Silicon/Intel、Windows x64、Linux x64。只做本地及模拟接口验证，尚未执行真实云效写入。原计划保存在 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 启动

安装 Node.js 24+、Rust stable，以及 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)。ADB 使用官方 Android SDK Platform Tools，不嵌入未经验证的第三方二进制。

```sh
npm ci
npm run dev
```

```sh
npm run check
npm run bundle:local # 本机 macOS：从首次构建带签名，并自动校验应用和 DMG
```

Linux 编译需要 WebKitGTK 4.1、GTK 3、DBus、librsvg、patchelf 和 OpenSSL 开发包。保存 PAT 需要可用且已解锁的 Secret Service；否则选择“仅本次使用”。macOS 使用钥匙串，Windows 使用系统凭据库。PAT 不存入 SQLite 或项目配置。

本机打包、验证复用、界面失败诊断及覆盖安装步骤见 [本地开发与打包约定](docs/local-workflow.md)。本机 macOS 打包只生成当前架构的开发包，不自动安装；Windows/Linux 和正式候选继续使用现有平台/Actions 流程。

## 结构

| 目录                | 用途                                                 |
| ------------------- | ---------------------------------------------------- |
| `src/renderer`      | 中文授权、采集、问题表单、附件预览与提交恢复界面     |
| `src/shared`        | 草稿契约与界面校验                                   |
| `src-tauri/src`     | Rust ADB、凭据、SQLite、证据 ZIP、云效 REST 和状态机 |
| `profiles`          | 照片恢复PR启动配置，不包含 PAT、负责人或版本 ID      |
| `schemas`           | 草稿、配置和证据 manifest JSON Schema                |
| `tests/fixtures`    | 仅用于测试的项目、成员、版本和草稿数据               |
| `.github/workflows` | 检查与四个原生目标的打包配置                         |
| `docs`              | 任务拆解、操作说明、接口契约、阶段验收               |

## GitHub Actions

推送到 `main/master` 或创建 PR 执行检查。手动运行 **Desktop bundles** 或推送 `v*` tag 构建安装包，并保存为 Actions artifacts；不自动创建或发布 GitHub Release。

macOS 构建使用 ad-hoc 签名；企业证书、Apple 公证及 Windows 签名后续接入。Linux x64 输出 `.deb` 与 `.AppImage`，Windows x64 输出 NSIS `.exe`，macOS 两架构分别输出 `.dmg` 与 `.app.zip`。其他架构可继续扩展矩阵。

远程仓库是 [guozhiqiang123/AndroidBugDesk](https://github.com/guozhiqiang123/AndroidBugDesk)。任务成果、版本候选、主分支合并和 Actions 构建按 [合并与推送约定](docs/git-delivery-policy.md) 推进；首次源码上传先完成公开历史检查。正式发布单独确认。

## 当前阶段

已提供可构建的本地工作台与模拟提交闭环，完整验收仍按原计划第 13 节执行。真实云效 PNG/JPEG/MP4/ZIP 上传、目标系统 USB、凭据存储、媒体播放、安装包签名尚需验收。

继续实施从 [任务清单](docs/tasks.md) 和 [验收记录](docs/acceptance.md) 开始。不要将编译成功或模拟接口通过标成真实产品验收。

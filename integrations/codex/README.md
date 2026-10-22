# Codex 接入 Android Bug Desk

推荐在已安装客户端中打开“接入 AI”，点击“复制给 Codex”，再把完整提示语粘贴到 Codex。提示语会包含当前系统的真实程序路径和客户端内置 Skill 路径，Codex 可自动完成下列配置。

手工接入步骤：

1. 启动 Android Bug Desk，完成登录和项目选择。
2. 在“设置 → 开发者选项”中开启“允许本机 AI 接入”。
3. 将 [`mcp-config.example.json`](./mcp-config.example.json) 中的 `command` 替换为当前系统的已安装可执行文件。
4. 将 [`skills/android-bug-submission`](./skills/android-bug-submission) 安装为 Codex Skill。

常见安装路径：

- macOS：`/Applications/Android Bug Desk.app/Contents/MacOS/android-bug-desk`
- Windows：`%LOCALAPPDATA%\Android Bug Desk\android-bug-desk.exe`
- Linux deb：`/usr/bin/android-bug-desk`
- Linux AppImage：填写用户实际保存的 AppImage 绝对路径

MCP 服务必须传入 `--mcp-bridge`。bridge 本身不读取云效凭据，只连接正在运行的桌面客户端；客户端关闭或关闭 AI 接入开关后，当前连接令牌立即失效。

开发构建可将 `command` 指向 `src-tauri/target/debug/android-bug-desk`。测试隔离时可以给 bridge 设置 `ANDROID_BUG_DESK_DATA_DIR`，其值必须是绝对路径，并与被测试客户端使用同一个目录。

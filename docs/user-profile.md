# 菜单底部的云效用户信息

2026-10-09 官方文档核对：

- GetUserByToken：`GET /oapi/v1/platform/user`，`x-yunxiao-token` 为个人令牌；权限为组织管理 / 用户 / 只读。响应有 `id`、`name`、`nickName`、`lastOrganization` 等字段，未声明头像字段。
  https://help.aliyun.com/zh/yunxiao/developer-reference/getuserbytoken
- GetUserInfo：`GET /users/current`，Codeup 侧响应有 `avatarUrl`、`name`、`username`。这是另一套接口；不能将其路径或返回契约直接套用到当前平台登录接口。
  https://help.aliyun.com/zh/yunxiao/developer-reference/api-devops-2021-06-25-getuserinfo/

菜单底部展示当前验证成功的用户。优先使用 nickName，空白时回退 name，均无值时显示“云效用户”。用户 ID 和邮箱不用于占位显示。

当前登录接口没有文档承诺的头像字段，不能保证读取到真实头像。代码仅兼容当前用户响应实际返回的 avatarUrl / avatar；不新增推测接口，不按昵称匹配其他成员或复用其他用户的资料。未返回头像时显示昵称首字。

若响应实际带回头像，只允许已在官方头像示例出现的 HTTPS tcs-devops.aliyuncs.com /thumbnail/ 地址。Rust 用独立无凭据客户端下载，禁止重定向，4 秒超时，最多 512 KB；仅解码 PNG/JPEG，限制尺寸与解码内存并缩放到最长边 128 px、重新编码 PNG。渲染端只接收 PNG 字节并以本地 data URL 显示；不开放 renderer 网络权限，不向头像服务器转发 PAT。头像失败不影响登录。

用户资料仅随当前登录状态保存在内存。账号变化、退出或授权失效时清除旧展示。启动依然为空令牌输入页；仅点击“确定”后验证和获取用户资料。

## 本轮验证

- npm run check：50 项前端、35 项 Rust 通过，真实只读测试 1 项 ignored，TypeScript/Vite 通过。
- Rust fmt 和严格 Clippy 通过。
- mock 验证昵称字段、无头像时不增加请求、头像下载不带令牌、不跟随跳转、拒绝不可信地址/过大数据/SVG/无效图像。
- 浏览器模拟 Tauri IPC：昵称与本地图片、导航底部位置、401 清除用户、第二账号替换、图片失败回退；原 11 项登录/表单/历史模拟场景继续通过。
- 未调用真实个人 PAT，未验证官方实际头像响应；截图为模拟用户。未构建新安装包、未公开源码、未运行 Actions。

基线为已经技术接受的账号恢复返修 e2754e7；本次独立 worktree 仅更改用户资料字段、解析、头像下载与侧栏展示，不替代 GZQ-49 完整集成或人工 UI 验收。

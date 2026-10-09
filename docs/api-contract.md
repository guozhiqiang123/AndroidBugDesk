# P0 接口契约记录

核对日期：2026-10-09。模式：中心版。接入点固定为 `https://openapi-rdc.aliyuncs.com`，PAT 仅置于 `x-yunxiao-token` 敏感请求头。禁止向 Jenkins 或签名附件下载地址转发此头，禁止自动跟随接口重定向。

组织前缀 `/oapi/v1/projex/organizations/{organizationId}`。

| 接口                                                                                                                                                                                 | 实施与契约                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| [类型](https://help.aliyun.com/zh/yunxiao/developer-reference/listworkitemtypes)、[字段](https://help.aliyun.com/zh/yunxiao/developer-reference/getworkitemtypefieldconfig)          | Bug 类型；字段 required、type、format、选项；未知必填格式停止写入                                                                    |
| [成员](https://help.aliyun.com/zh/yunxiao/developer-reference/listprojectmembers)                                                                                                    | userId/userName，同名靠 ID 区分                                                                                                      |
| [版本](https://help.aliyun.com/zh/yunxiao/developer-reference/listversions)、[迭代](https://help.aliyun.com/zh/yunxiao/developer-reference/listsprints)                              | page/perPage 与 x-next-page/x-total-pages，完整分页                                                                                  |
| [创建](https://help.aliyun.com/zh/yunxiao/developer-reference/createworkitem)                                                                                                        | subject、spaceId、workitemTypeId、assignedTo；MARKDOWN；customFieldValues 多值转逗号分隔；不指定 operatorId 或状态                   |
| [详情](https://help.aliyun.com/zh/yunxiao/developer-reference/getworkitem)                                                                                                           | 项目来自 space.id；编号来自 serialNumber；官方详情示例未承诺返回真实 URL                                                             |
| [更新](https://help.aliyun.com/zh/yunxiao/developer-reference/updateworkitem)                                                                                                        | PUT description/formatType；只替换唯一受管理证据段落；回读核对。API 无跨请求原子合并，第二次读取可发现部分并发编辑，仍需实测竞争窗口 |
| [上传](https://help.aliyun.com/zh/yunxiao/developer-reference/createworkkitemattachment)、[附件列表](https://help.aliyun.com/zh/yunxiao/developer-reference/listworkitemattachments) | multipart file；自动生成 boundary；上传 ID 与列表 id/fileId 核对，下载 SHA256 核对；下载客户端无 PAT                                 |
| [搜索](https://help.aliyun.com/zh/yunxiao/developer-reference/searchworkitems)                                                                                                       | 项目内标题候选后逐条详情核对唯一 evidenceId；搜索未命中不等于未创建                                                                  |

| 媒体     | 本地与 mock                                             | 真实上传/下载 | 云效页面显示/播放                                | API 大小上限 |
| -------- | ------------------------------------------------------- | ------------- | ------------------------------------------------ | ------------ |
| PNG/JPEG | 文件头与图片解码校验、预览接口、SHA256 已实施           | 未验收        | 未验收                                           | 未确定       |
| MP4      | ftyp/moov/mdat 容器结构校验、原文件上传与哈希流程已实施 | 未验收        | 未验收；Linux 播放依赖 WebKit/GStreamer 解码环境 | 未确定       |
| ZIP      | 生成、回读、哈希稳定已测试                              | 未验收        | 未验收                                           | 未确定       |

50 MB 为本地提示阈值，不能视为云效服务端上限。未经真实组织验证，不宣称视频上传或在线播放已验收。成功链接仅接受服务端返回的 devops.aliyun.com HTTPS URL；实际链接解析将通过验收项目页面确认后补全。

此记录没有使用现有协调员令牌，没有创建真实缺陷。

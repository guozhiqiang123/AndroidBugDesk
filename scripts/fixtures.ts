import { format } from "prettier";
import { writeFileSync, mkdirSync } from "node:fs";
import { emptyDraft, type Metadata, type Profile } from "../src/shared/domain";
const p: Profile = {
  name: "模拟验收项目",
  edition: "central",
  organizationId: "test-org",
  projectId: "test-project",
  businessProfiles: ["测试方向"],
  requiredFieldIds: [],
  fieldMappings: {},
  enableVersions: true,
  enableSprints: true,
  attachmentWarningMb: 50,
};
const d = emptyDraft(
  "00000000-0000-4000-8000-000000000001",
  "2026-10-09T00:00:00Z",
);
Object.assign(d, {
  title: "点击恢复按钮后页面闪退",
  business: "测试方向",
  workitemTypeId: "bug-type",
  assignedTo: "member-1",
  versions: ["version-2"],
  buildUrl: "https://jenkins.example/job/App/42/",
  buildStatus: "manual_confirmed",
  manualNote: "设备设置页面，测试人员核对",
  preconditions: "新安装；允许照片权限；Wi-Fi 正常",
  steps: "进入照片恢复首页\n点击开始扫描按钮\n选择一张图片并点击恢复",
  expected: "图片恢复并显示完成提示",
  actual: "页面闪退，回到桌面",
  previewConfirmed: true,
});
d.app = {
  packageName: "com.example.test",
  versionName: "1.0",
  versionCode: "0",
  channel: "internal",
  environment: "测试包",
  obfuscation: "已混淆，构建配置确认",
  hardening: "未加固，构建配置确认",
};
d.device = {
  model: "模拟设备",
  androidVersion: "14",
  totalMemKb: "8000000",
  availableMemKb: "2000000",
  resolution: "1080x2400",
  density: "420",
};
d.event = {
  occurredAt: "2026-10-09T08:00:00+08:00",
  originalAt: "",
  source: "manual",
  sourceNote: "测试人员操作记录",
  lateCapture: false,
};
d.cloud = {
  applicability: "no",
  reason: "该入口为本地固定功能，应用调试面板确认没有开关",
  name: "",
  effectiveValue: "",
  source: "",
  observedAt: "",
  override: "",
};
d.customFields = { priority: "p2" };
d.attachments = [
  {
    id: "00000000-0000-4000-8000-000000000002",
    name: "logcat.log",
    path: "/fixture/logcat.log",
    mime: "text/plain",
    size: 42,
    sha256: "a".repeat(64),
    role: "log",
    source: "导入测试日志",
    coverageStart: "2026-10-09T07:59:00+08:00",
    coverageEnd: "2026-10-09T08:01:00+08:00",
    confirmed: true,
    derivedFrom: null,
  },
];
const m: Metadata = {
  profile: p,
  typeId: "bug-type",
  types: [{ id: "bug-type", name: "缺陷" }],
  fields: [
    {
      id: "priority",
      name: "优先级",
      type: "SystemCustomField",
      format: "list",
      required: true,
      options: [{ id: "p2", name: "普通" }],
    },
  ],
  members: [{ id: "member-1", name: "同名成员" }],
  versions: [
    { id: "version-1", name: "同名版本" },
    { id: "version-2", name: "同名版本" },
  ],
  sprints: [],
  fetchedAt: "2026-10-09T00:00:00Z",
};
mkdirSync("tests/fixtures", { recursive: true });
for (const [name, value] of Object.entries({ "valid-draft": d, metadata: m }))
  writeFileSync(
    `tests/fixtures/${name}.json`,
    await format(JSON.stringify(value), { parser: "json" }),
  );

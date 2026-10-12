import { describe, expect, it } from "vitest";
import draft from "./fixtures/valid-draft.json";
import meta from "./fixtures/metadata.json";
import { draftSchema, type Draft, type Metadata } from "../src/shared/domain";
import { parseBuildUrl, validateDraft } from "../src/shared/validation";
import { excludedField } from "../src/shared/submission-fields";
const d = () => structuredClone(draft) as Draft;
const m = () => structuredClone(meta) as Metadata;
const check = (draft: Draft, metadata = m()) =>
  validateDraft(draft, metadata.profile, metadata, true);
describe("提交门槛", () => {
  it("事后补采字段已移除，草稿直接使用新结构", () => {
    const v = d();
    expect(v.event).not.toHaveProperty("lateCapture");
    expect(draftSchema.safeParse(v).success).toBe(true);
    expect(check(v)).toEqual([]);
    Object.assign(v.event, { lateCapture: true });
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("加固状态已移除，不阻断提交且旧属性被格式校验拒绝", () => {
    const v = d();
    expect(v.app).not.toHaveProperty("hardening");
    expect(draftSchema.safeParse(v).success).toBe(true);
    expect(check(v)).toEqual([]);
    Object.assign(v.app, { hardening: "旧加固状态" });
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("有效信息与 versionCode 0 通过", () => expect(check(d())).toEqual([]));
  it.each([undefined, "", "com.example.test.RecoveryActivity"])(
    "Bug 所在页面为 %s 时保持可选且兼容已有草稿",
    (bugPage) => {
      const v = d();
      if (bugPage === undefined) delete v.bugPage;
      else v.bugPage = bugPage;
      expect(draftSchema.safeParse(v).success).toBe(true);
      expect(check(v)).toEqual([]);
      expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
    },
  );
  it("草稿不再包含测试环境，旧字段被拒绝", () => {
    const v = d();
    expect(v.app).not.toHaveProperty("environment");
    expect(draftSchema.safeParse(v).success).toBe(true);
    expect(check(v)).toEqual([]);
    expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
    Object.assign(v.app, { environment: "旧测试包" });
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("混淆状态已移除，不再要求补充，旧属性被拒绝", () => {
    const v = d();
    expect(v.app).not.toHaveProperty("obfuscation");
    expect(draftSchema.safeParse(v).success).toBe(true);
    expect(check(v)).toEqual([]);
    Object.assign(v.app, { obfuscation: "已混淆" });
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it.each([
    "jenkinsPackageUrl",
    "title",
    "workitemTypeId",
    "assignedTo",
    "actual",
  ])("缺少 %s 阻止提交", (path) => {
    const v = d();
    const parts = path.split(".");
    let obj = v as unknown as Record<string, unknown>;
    for (const key of parts.slice(0, -1))
      obj = obj[key] as Record<string, unknown>;
    obj[parts.at(-1)!] = "";
    expect(check(v).length).toBeGreaterThan(0);
  });
  it.each([
    ["jenkinsPackageUrl", "Jenkins包地址"],
    ["title", "问题标题"],
    ["actual", "问题表现"],
  ] as const)("%s 只有空白时在线与离线核对均阻止提交", (path, label) => {
    const v = d();
    v[path] = " \n\t ";
    for (const online of [true, false]) {
      const issues = validateDraft(v, m().profile, m(), true, online);
      expect(
        issues.some(
          (issue) => issue.field === path && issue.message.includes(label),
        ),
      ).toBe(true);
    }
  });
  it.each(["", "2026-10-09T08:00:00", "旧时间记录"])(
    "移除人工时间输入后内部时间 %j 及空依据不阻止核对",
    (occurredAt) => {
      const v = d();
      v.event.occurredAt = occurredAt;
      v.event.originalAt = "2026-10-09T08:00:00+08:00";
      v.event.sourceNote = "";
      v.event.source = "manual";
      expect(draftSchema.safeParse(v).success).toBe(true);
      expect(check(v)).toEqual([]);
      expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
    },
  );
  it("问题标题只要求有效内容，不再要求五个字符", () => {
    const v = d();
    v.title = "闪退";
    expect(check(v)).toEqual([]);
  });
  it.each(["preconditions", "steps", "expected"] as const)(
    "%s 留空或填写占位内容时均不阻止提交",
    (path) => {
      for (const value of ["", " \n\t ", "未知"]) {
        const v = d();
        v[path] = value;
        expect(check(v)).toEqual([]);
        expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
      }
    },
  );
  it("选填的复现步骤内容不受主观字数门槛", () => {
    const v = d();
    v.steps = "3331111\n444\n555\n666";
    expect(check(v)).toEqual([]);
    expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
  });
  it.each(["", " ", "\n\t"])("项目包名为空 %j 时阻止核对", (blank) => {
    const v = d();
    v.app.packageName = blank;
    expect(check(v)).toContainEqual({
      field: "app.packageName",
      message: "请补充当前项目的 Android 包名配置",
    });
    expect(validateDraft(v, m().profile, null, false, false)).toEqual(check(v));
  });
  it("包名存在时设备字段和 Bug 页面可以全部为空", () => {
    const v = d();
    delete v.bugPage;
    v.app.versionName = "";
    v.app.versionCode = "";
    for (const key of Object.keys(v.device))
      v.device[key as keyof Draft["device"]] = "";
    expect(check(v)).toEqual([]);
  });
  it.each([
    "https://ci/job/App/",
    "https://ci/job/App/lastBuild/",
    "https://ci/job/App/lastSuccessfulBuild/",
    "https://ci/job/App/42/?token=x",
    "file:///job/App/42/",
    "https://name:pass@ci/job/App/42/",
  ])("拒绝动态或不安全构建链接 %s", (url) =>
    expect(parseBuildUrl(url)).toBeNull(),
  );
  it("多级 job 固定链接可解析", () =>
    expect(parseBuildUrl("http://ci/job/Folder/job/App/123/")).toEqual({
      job: "/job/Folder/job/App",
      number: "123",
    }));
  it.each([
    "功能",
    "UI",
    "多语言",
    "动态过程",
    "崩溃",
    "ANR",
    "性能",
    "网络",
    "数据",
  ] as const)("%s 的附件可为空", (kind) => {
    const v = d();
    v.kind = kind;
    v.attachments = [];
    Object.assign(v.network, {
      operation: "请求接口",
      type: "Wi-Fi",
      error: "超时",
      collectionStatus: "已记录",
    });
    Object.assign(v.data, { type: "图片", count: "1", source: "本地测试数据" });
    expect(check(v)).toEqual([]);
    expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
  });
  it("图片和带旧确认标记的有效日志都能通过", () => {
    for (const attachments of [
      [{ ...d().attachments[0], role: "image" as const }],
      [
        {
          ...d().attachments[0],
          confirmed: false,
        },
      ],
    ]) {
      const v = d();
      v.attachments = attachments;
      expect(check(v)).toEqual([]);
    }
  });
  it("每份日志都校验，旧确认标记不能绕过错误", () => {
    const v = d();
    const invalid = {
      ...v.attachments[0],
      id: "invalid-log",
      size: 0,
      source: "",
      coverageStart: "",
      coverageEnd: "",
    };
    v.attachments.push(invalid);
    expect(
      check(v).filter((e) => e.field.startsWith("attachment.invalid-log.")),
    ).toHaveLength(3);
    invalid.confirmed = false;
    expect(
      check(v).filter((e) => e.field.startsWith("attachment.invalid-log.")),
    ).toHaveLength(3);
  });
  it("占位问题信息被阻止，未选择日志和人工时间不设门槛", () => {
    const v = d();
    v.event.occurredAt = "2026-10-09T08:00:00";
    v.actual = "未知";
    v.attachments[0].confirmed = false;
    expect(check(v).map((issue) => issue.field)).toEqual(["actual"]);
    expect(check(v).some((issue) => issue.field.startsWith("event."))).toBe(
      false,
    );
  });
  it("云效新增必填字段或未支持格式被阻止", () => {
    for (const format of ["string", "cascading"]) {
      const metadata = m();
      metadata.fields.push({
        id: "new-required",
        name: "新字段",
        format,
        type: "CustomField",
        required: true,
        options: [],
      });
      expect(
        check(d(), metadata).some(
          (e) => e.field === "customFields.new-required",
        ),
      ).toBe(true);
    }
  });
  it("过期成员、版本及枚举选项被阻止", () => {
    const v = d();
    v.versions = ["stale"];
    v.assignedTo = "stale";
    v.customFields.priority = "stale";
    expect(check(v).length).toBeGreaterThanOrEqual(3);
  });
  it.each([
    ["business", "旧业务方向"],
    ["manualNote", "旧人工补充来源"],
    ["buildUrl", "https://jenkins.example/job/App/42/"],
    ["buildStatus", "manual_confirmed"],
    ["buildManifest", {}],
  ])("已移除字段 %s 被草稿格式拒绝", (key, value) => {
    const v = d() as Draft & Record<string, unknown>;
    v[key] = value;
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("已移除的渠道字段被草稿格式拒绝", () => {
    const v = d() as Draft & { app: Draft["app"] & { channel?: string } };
    v.app.channel = "internal";
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("设备有效云控硬切移除，在线和离线校验均无需云控，旧属性被拒绝", () => {
    const v = d();
    expect(v).not.toHaveProperty("cloud");
    expect(draftSchema.safeParse(v).success).toBe(true);
    expect(check(v)).toEqual([]);
    expect(validateDraft(v, m().profile, null, false, false)).toEqual([]);
    Object.assign(v, {
      cloud: {
        applicability: "no",
        reason: "旧云控依据",
        name: "",
        effectiveValue: "",
        source: "",
        observedAt: "",
        override: "",
      },
    });
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
  it("日志覆盖时间按时区比较", () => {
    const v = d();
    v.attachments[0].coverageStart = "2026-10-09T07:59:00+08:00";
    v.attachments[0].coverageEnd = "2026-10-09T00:01:00Z";
    expect(check(v)).toEqual([]);
  });
  it("校验跨进程草稿契约", () => {
    expect(draftSchema.safeParse(d()).success).toBe(true);
    const v = d();
    v.attachments[0].sha256 = "wrong";
    expect(draftSchema.safeParse(v).success).toBe(false);
  });
});

describe("主页移除的云效字段", () => {
  const field = (id: string, name: string, required = false) => ({
    id,
    name,
    format: "list",
    type: "CustomField",
    required,
    options: [],
  });
  it.each([
    ["labels", "其他名称"],
    ["tags", "其他名称"],
    ["plannedStartDate", "其他名称"],
    ["plannedEndTime", "其他名称"],
    ["notFixReason", "其他名称"],
    ["custom-label", "标签"],
    ["custom-start", "计划开始时间"],
    ["custom-end", "计划完成时间"],
    ["custom-reason", "不修复理由"],
    ["estimatedTime", "其他名称"],
    ["custom-estimate", "预计工时"],
  ])("忽略已移除的可选字段旧值 %s", (id, name) => {
    const metadata = m();
    metadata.fields.push(field(id, name));
    const v = d();
    v.customFields[id] = "stale";
    expect(check(v, metadata)).toEqual([]);
  });
  it("识别配置映射，并忽略已消失的已移除字段旧值", () => {
    const metadata = m();
    metadata.profile.fieldMappings["mapped-label"] = "labels";
    const v = d();
    v.customFields["mapped-label"] = "stale";
    v.customFields.plannedStartTime = "invalid-date";
    expect(
      excludedField(field("mapped-label", "自定义分类"), metadata.profile),
    ).toBe(true);
    expect(check(v, metadata)).toEqual([]);
    v.customFields["unknown"] = "stale";
    expect(
      check(v, metadata).some(
        (issue) => issue.field === "customFields.unknown",
      ),
    ).toBe(true);
  });
  it.each(["metadata", "profile"])(
    "%s 声明移除字段为必填时报告配置错误",
    (source) => {
      const metadata = m();
      metadata.fields.push(field("labels", "标签", source === "metadata"));
      if (source === "profile")
        metadata.profile.requiredFieldIds.push("labels");
      const v = d();
      v.customFields.labels = "stale";
      expect(check(v, metadata)).toEqual([
        {
          field: "customFields.labels",
          message: "云效项目将已移除字段「标签」设为必填，请调整项目配置后提交",
        },
      ]);
    },
  );
  it("保留近似名称字段的校验", () => {
    const metadata = m();
    for (const [id, name] of [
      ["estimate-note", "预计工时说明"],
      ["label-note", "标签说明"],
    ]) {
      const f = { ...field(id, name, true), format: "string" };
      metadata.fields.push(f);
      expect(excludedField(f, metadata.profile)).toBe(false);
    }
    expect(check(d(), metadata).map((issue) => issue.field)).toEqual([
      "customFields.estimate-note",
      "customFields.label-note",
    ]);
  });
  it("预计工时被声明必填时仍报告项目配置错误", () => {
    const metadata = m();
    metadata.fields.push(field("estimatedTime", "预计工时", true));
    expect(check(d(), metadata)).toEqual([
      {
        field: "customFields.estimatedTime",
        message:
          "云效项目将已移除字段「预计工时」设为必填，请调整项目配置后提交",
      },
    ]);
  });
  it("忽略云效隐藏的可选旧值，但保留项目声明的必填字段", () => {
    const metadata = m();
    metadata.fields.push({
      ...field("hidden", "已隐藏字段"),
      showWhenCreate: false,
    });
    const v = d();
    v.customFields.hidden = "stale";
    expect(check(v, metadata)).toEqual([]);
    metadata.profile.requiredFieldIds.push("hidden");
    expect(
      check(v, metadata).some((issue) => issue.field === "customFields.hidden"),
    ).toBe(true);
  });
});

describe("派生回执完整性", () => {
  it("窗口、脱敏和来源回执成组确认；缺回执阻止提交", () => {
    const v = d();
    const original = v.attachments[0];
    const window = {
      ...original,
      id: "00000000-0000-4000-8000-000000000003",
      source: "ADB 问题日志窗口派生副本；未脱敏",
      derivedFrom: original.id,
    };
    const receipt = {
      ...original,
      id: "00000000-0000-4000-8000-000000000004",
      role: "report" as const,
      source: "日志窗口范围与缺口记录",
      derivedFrom: window.id,
    };
    v.attachments.push(window, receipt);
    expect(check(v)).toEqual([]);
    receipt.confirmed = false;
    expect(check(v)).toEqual([]);
    receipt.confirmed = true;
    const redacted = {
      ...window,
      id: "00000000-0000-4000-8000-000000000005",
      source: "文本派生副本：仅按提供的精确文本替换；未保证完整脱敏",
      derivedFrom: window.id,
    };
    const textReceipt = {
      ...receipt,
      id: "00000000-0000-4000-8000-000000000006",
      source: "文本替换记录（请核对副本）",
      derivedFrom: redacted.id,
    };
    v.attachments.push(redacted, textReceipt);
    expect(check(v).some((e) => e.message.includes("来源回执"))).toBe(true);
    const inherited = {
      ...receipt,
      id: "00000000-0000-4000-8000-000000000007",
      derivedFrom: redacted.id,
    };
    v.attachments.push(inherited);
    expect(check(v)).toEqual([]);
    v.attachments = v.attachments.filter((a) => a.id !== redacted.id);
    expect(check(v).some((e) => e.message.includes("关联副本"))).toBe(true);
  });
});

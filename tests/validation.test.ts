import { describe, expect, it } from "vitest";
import draft from "./fixtures/valid-draft.json";
import meta from "./fixtures/metadata.json";
import { draftSchema, type Draft, type Metadata } from "../src/shared/domain";
import { parseBuildUrl, validateDraft } from "../src/shared/validation";
const d = () => structuredClone(draft) as Draft;
const m = () => structuredClone(meta) as Metadata;
const check = (draft: Draft, metadata = m()) =>
  validateDraft(draft, metadata.profile, metadata, true);
describe("提交门槛", () => {
  it("有效信息与 versionCode 0 通过", () => expect(check(d())).toEqual([]));
  it.each([
    "title",
    "business",
    "workitemTypeId",
    "assignedTo",
    "buildUrl",
    "preconditions",
    "steps",
    "expected",
    "actual",
    "app.packageName",
    "app.versionName",
    "app.versionCode",
    "app.channel",
    "app.environment",
    "app.obfuscation",
    "app.hardening",
    "device.model",
    "device.androidVersion",
    "device.totalMemKb",
    "device.availableMemKb",
    "device.resolution",
    "device.density",
    "event.occurredAt",
    "event.sourceNote",
    "manualNote",
    "cloud.reason",
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
  it("日志不能用截图或空文件替代", () => {
    for (const attachments of [
      [],
      [{ ...d().attachments[0], size: 0 }],
      [{ ...d().attachments[0], role: "image" as const }],
    ]) {
      const v = d();
      v.attachments = attachments;
      expect(check(v).some((e) => e.field === "attachments")).toBe(true);
    }
  });
  it("缺少时区、占位信息、未确认日志被阻止", () => {
    const v = d();
    v.event.occurredAt = "2026-10-09T08:00:00";
    v.app.channel = "未知";
    v.attachments[0].confirmed = false;
    expect(check(v).length).toBeGreaterThanOrEqual(3);
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
  it("构建身份冲突被阻止", () => {
    const v = d();
    v.buildManifest = {
      packageName: "com.other.app",
      versionName: "1.0",
      versionCode: "0",
      commit: null,
      artifactSha256: null,
    };
    expect(check(v).some((e) => e.field === "buildStatus")).toBe(true);
  });
  it("云控适用时必须是设备生效证据", () => {
    const v = d();
    v.cloud.applicability = "yes";
    v.cloud.source = "后台配置截图";
    expect(check(v).some((e) => e.field === "cloud.source")).toBe(true);
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

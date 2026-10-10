import { describe, expect, it } from "vitest";
import fixture from "./fixtures/valid-draft.json";
import meta from "./fixtures/metadata.json";
import type { Draft, Metadata } from "../src/shared/domain";
import { validateDraft } from "../src/shared/validation";
import {
  routeIssues,
  attachmentIssueKey,
  groupIssuesByNavigationTarget,
} from "../src/renderer/issue-routing";
const fresh = () => structuredClone(fixture) as Draft;
const routes = (draft: Draft) =>
  routeIssues(
    draft,
    validateDraft(draft, (meta as Metadata).profile, meta as Metadata, true),
  );
describe("字段提示归属", () => {
  it("映射到同一输入项的本地与云效校验只生成一个核对入口", () => {
    const groups = groupIssuesByNavigationTarget(
      [
        { field: "title", message: "请补充问题标题" },
        {
          field: "customFields.subject-id",
          message: "请补充云效必填字段「标题」",
        },
        { field: "actual", message: "请补充问题表现" },
        {
          field: "customFields.description-id",
          message: "请补充云效必填字段「描述」",
        },
      ],
      (field) =>
        field === "customFields.subject-id"
          ? "field-title"
          : field === "customFields.description-id"
            ? "field-actual"
            : `field-${field}`,
    );

    expect(groups).toEqual([
      {
        target: "field-title",
        field: "title",
        messages: ["请补充问题标题", "请补充云效必填字段「标题」"],
      },
      {
        target: "field-actual",
        field: "actual",
        messages: ["请补充问题表现", "请补充云效必填字段「描述」"],
      },
    ]);
  });

  it("同一输入项的完全重复错误不会重复计数", () => {
    expect(
      groupIssuesByNavigationTarget(
        [
          { field: "title", message: "请补充问题标题" },
          { field: "title", message: "请补充问题标题" },
        ],
        (field) => `field-${field}`,
      ),
    ).toEqual([
      {
        target: "field-title",
        field: "title",
        messages: ["请补充问题标题"],
      },
    ]);
  });

  it("动态过程不要求导入图像或视频", () => {
    const d = fresh();
    d.kind = "动态过程";
    const r = routes(d);
    expect(r["attachment-import"]).toBeUndefined();
  });
  it("只有有问题的日志覆盖终点显示逆序错误", () => {
    const d = fresh();
    const log = d.attachments[0];
    log.coverageEnd = "2026-10-09T01:00:00+08:00";
    log.coverageStart = "2026-10-09T02:00:00+08:00";
    const r = routes(d);
    expect(r[attachmentIssueKey(log, "coverageEnd")]).toHaveLength(1);
    expect(r[attachmentIssueKey(log, "coverageStart")]).toBeUndefined();
    expect(r["attachment-import"]).toBeUndefined();
  });
  it("旧确认标记不再改变有效日志的校验", () => {
    const d = fresh();
    d.attachments[0].confirmed = false;
    const r = routes(d);
    expect(
      r[attachmentIssueKey(d.attachments[0], "confirmed")],
    ).toBeUndefined();
    expect(
      r[attachmentIssueKey(d.attachments[0], "coverageEnd")],
    ).toBeUndefined();
  });
  it("回执问题只标记相关副本，不标记其他已确认日志", () => {
    const d = fresh();
    const raw = d.attachments[0];
    const copy = {
      ...raw,
      id: "copy",
      derivedFrom: raw.id,
      source: "ADB 问题日志窗口派生副本；未脱敏",
    };
    d.attachments.push(copy);
    const r = routes(d);
    expect(r[attachmentIssueKey(copy, "confirmed")]).toContain(
      "派生副本必须携带全部来源回执",
    );
    expect(r[attachmentIssueKey(raw, "confirmed")]).toBeUndefined();
  });
  it("未知附件校验仍有可见回退位置", () => {
    expect(
      routeIssues(fresh(), [
        { field: "attachments", message: "服务端附件规则变化" },
      ]),
    ).toEqual({ "attachment-import": ["服务端附件规则变化"] });
  });
});

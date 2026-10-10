import { describe, expect, it } from "vitest";
import fixture from "./fixtures/valid-draft.json";
import meta from "./fixtures/metadata.json";
import type { Draft, Metadata, Submission } from "../src/shared/domain";
import {
  draftHistoryValues,
  emptyHistoryFilters,
  historyOptions,
  localHistoryRows,
} from "../src/shared/history";
const metadata = meta as Metadata;
const draft = (): Draft => structuredClone(fixture) as Draft;
const submission = (
  d: Draft,
  overrides: Partial<Submission> = {},
): Submission =>
  ({
    draftId: d.id,
    profile: metadata.profile,
    state: "submitted",
    identifier: "BUG-100",
    workitemId: "work-a",
    updatedAt: "2026-10-10T10:00:00Z",
    ...overrides,
  }) as Submission;
describe("历史记录筛选", () => {
  it("只展示主页面的组织/项目，不变更草稿或提交", () => {
    const d = draft();
    const s = submission(d);
    const otherProject = submission(d, {
      profile: { ...metadata.profile, projectId: "other" },
    });
    const otherOrg = submission(d, {
      profile: { ...metadata.profile, organizationId: "other" },
    });
    const before = structuredClone([s, otherProject, otherOrg, d]);
    expect(
      localHistoryRows(
        [s, otherProject, otherOrg],
        [d],
        metadata.profile,
        emptyHistoryFilters(),
        metadata,
      ),
    ).toEqual([s]);
    expect([s, otherProject, otherOrg, d]).toEqual(before);
  });
  it("九项条件取交集，每项多选取并集，优先级/角色支持映射 ID", () => {
    const m = structuredClone(metadata);
    m.profile.fieldMappings = {
      verifyAlias: "verifier",
      participantAlias: "participants",
      ccAlias: "trackers",
    };
    m.fields.push(
      ...[
        { id: "verifyAlias", name: "验证者" },
        { id: "participantAlias", name: "参与者" },
        { id: "ccAlias", name: "抄送" },
        { id: "severityAlias", name: "严重程度" },
      ].map((f) => ({
        ...f,
        type: "CustomField",
        format: "list",
        required: false,
        options: [],
      })),
    );
    const d = draft();
    d.kind = "UI";
    d.assignedTo = "u1";
    d.sprint = "sp1";
    d.customFields = {
      priority: "p2",
      verifyAlias: "u2",
      participantAlias: ["u3", "u4"],
      ccAlias: ["u5"],
      severityAlias: "s2",
    };
    const s = submission(d);
    const filters = {
      ...emptyHistoryFilters(),
      kind: ["UI"],
      workitemTypeId: [d.workitemTypeId],
      assignedTo: ["u1"],
      sprint: ["sp1"],
      verifier: ["u2"],
      priority: ["p1", "p2"],
      severity: ["s2"],
      participants: ["u3"],
      trackers: ["u5"],
    };
    expect(localHistoryRows([s], [d], m.profile, filters, m)).toEqual([s]);
    expect(
      localHistoryRows(
        [s],
        [d],
        m.profile,
        { ...filters, participants: ["absent"] },
        m,
      ),
    ).toEqual([]);
    expect(draftHistoryValues(d, "verifier", m)).toEqual(["u2"]);
  });
  it("标题及工作项编号支持不区分大小写的搜索，缺失草稿仍可按编号找回", () => {
    const d = draft(),
      s = submission(d);
    expect(
      localHistoryRows(
        [s],
        [],
        metadata.profile,
        { ...emptyHistoryFilters(), keyword: " bug-100 " },
        metadata,
      ),
    ).toEqual([s]);
    expect(
      localHistoryRows(
        [s],
        [d],
        metadata.profile,
        { ...emptyHistoryFilters(), keyword: d.title.slice(0, 3) },
        metadata,
      ),
    ).toEqual([s]);
    expect(
      localHistoryRows(
        [s],
        [],
        metadata.profile,
        { ...emptyHistoryFilters(), kind: ["UI"] },
        metadata,
      ),
    ).toEqual([]);
  });
  it("选项保留旧记录使用的值并去重，排序优先最近更新", () => {
    const d = draft();
    d.customFields.priority = "retired-priority";
    expect(
      historyOptions("priority", metadata, [d, d]).map((v) => v.id),
    ).toEqual(["p2", "retired-priority"]);
    const old = submission(d, { draftId: "old", updatedAt: "2026-10-09" });
    const recent = submission(d, { draftId: "new", updatedAt: "2026-10-10" });
    expect(
      localHistoryRows(
        [old, recent],
        [],
        metadata.profile,
        emptyHistoryFilters(),
        metadata,
      ).map((v) => v.draftId),
    ).toEqual(["new", "old"]);
  });
});

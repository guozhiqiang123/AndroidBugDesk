import { describe, expect, it } from "vitest";
import {
  enqueueSelectionWrite,
  applySelectionMemory,
  rememberedValuesCompatible,
  profileConfigured,
  remoteAuthReaction,
  tabForAuth,
  initialWorkspace,
  initializeSelectionDefaults,
} from "../src/renderer/session";
import type { Draft, Metadata } from "../src/shared/domain";
import { nativeTarget } from "../src/shared/validation";
import {
  configuredProjectPackage,
  enabledProjects,
  normalizeProjectSettings,
  packageMismatch,
  projectsWithEnabledFirst,
} from "../src/renderer/project-settings";

describe("登录和工作区门禁", () => {
  it("识别真实 API 的角色字段和应用字段，不把自定义字段误映射", () => {
    const profile = {
      fieldMappings: {},
    } as import("../src/shared/domain").Profile;
    for (const [type, id] of [
      ["Role", "verifier"],
      ["Role", "participants"],
      ["Role", "trackers"],
      ["Application", "labels"],
      ["Application", "sprint"],
    ]) {
      expect(
        nativeTarget(
          { type, id } as import("../src/shared/domain").Field,
          profile,
        ),
      ).toBe(id);
    }
    expect(
      nativeTarget(
        {
          type: "SystemCustomField",
          id: "labels",
        } as import("../src/shared/domain").Field,
        profile,
      ),
    ).toBeUndefined();
  });
  it("只有服务端确认 valid 才进入新建提交", () => {
    expect(tabForAuth({ status: "valid" })).toBe("new");
    expect(tabForAuth({ status: "missing" })).toBe("auth");
    expect(tabForAuth({ status: "invalid" })).toBe("auth");
    expect(tabForAuth({ status: "unverified" })).toBe("auth");
  });

  it("只回填当前 API 仍提供的选择记忆", () => {
    expect(rememberedValuesCompatible("u1", ["u1", "u2"])).toBe(true);
    expect(rememberedValuesCompatible("deleted-user", ["u1", "u2"])).toBe(
      false,
    );
    expect(rememberedValuesCompatible(["v1", "v2"], ["v1", "v2", "v3"])).toBe(
      true,
    );
    expect(rememberedValuesCompatible(["v1", "deleted"], ["v1", "v2"])).toBe(
      false,
    );
    expect(rememberedValuesCompatible("u1", undefined)).toBe(true);
  });

  it("选择记忆写入按变更顺序串行执行并报告失败", async () => {
    const calls: string[] = [];
    let release!: () => void;
    const first = new Promise<void>((resolve) => {
      release = resolve;
    });
    let queue: Promise<unknown> = Promise.resolve();
    queue = enqueueSelectionWrite(
      queue,
      async () => {
        calls.push("first");
        await first;
      },
      () => undefined,
    );
    queue = enqueueSelectionWrite(
      queue,
      async () => {
        calls.push("second");
      },
      () => undefined,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual(["first"]);
    release();
    await queue;
    expect(calls).toEqual(["first", "second"]);

    const errors: unknown[] = [];
    await enqueueSelectionWrite(
      Promise.resolve(),
      async () => {
        throw new Error("offline");
      },
      (error) => errors.push(error),
    );
    expect(errors).toHaveLength(1);
  });

  it("新草稿应用记忆时保留无效选项和已有非选择字段", () => {
    const draft = {
      kind: "功能",
      workitemTypeId: "old",
      assignedTo: "",
      sprint: "",
      versions: [],
      customFields: { priority: "P1", untouched: "keep" },
    } as unknown as Draft;
    const metadata = {
      types: [{ id: "type-2", name: "Bug" }],
      members: [{ id: "u2", name: "用户" }],
      sprints: [],
      versions: [{ id: "v2", name: "版本" }],
      fields: [
        {
          id: "priority",
          name: "优先级",
          format: "list",
          type: "list",
          required: false,
          options: [{ id: "P1", name: "高" }],
        },
      ],
    } as unknown as Metadata;
    const next = applySelectionMemory(
      draft,
      {
        kind: "UI",
        workitemTypeId: "type-2",
        assignedTo: "u2",
        versions: ["deleted"],
        customFields: { priority: "P1", untouched: "replace" },
      },
      metadata,
    );
    expect(next.kind).toBe("UI");
    expect(next.assignedTo).toBe("u2");
    expect(next.versions).toEqual(["v2"]);
    expect(next.customFields.priority).toBe("P1");
    expect(next.customFields.untouched).toBe("keep");
  });

  it("启动只建立未授权登录页，不恢复凭据或项目", () => {
    const state = initialWorkspace();
    expect(state.authorized).toBe(false);
    expect(state.user).toBeNull();
    expect(state.metadata).toBeNull();
    expect(profileConfigured(state.profile)).toBe(false);
    expect(state.drafts).toEqual([]);
  });

  it.each([
    ["participants", "Role"],
    ["trackers", "Role"],
    ["participants", "NativeField"],
    ["trackers", "NativeField"],
    ["participants", "SystemCustomField"],
    ["trackers", "SystemCustomField"],
  ])("%s (%s) 默认空选且不回填选择记忆，保留草稿选择", (target, type) => {
    const id = type === "SystemCustomField" ? "mapped-role" : target;
    const profile = {
      ...initialWorkspace().profile,
      fieldMappings: { [id]: target },
    };
    const draft = {
      workitemTypeId: "",
      assignedTo: "",
      sprint: "",
      versions: [],
      customFields: {},
    } as unknown as Draft;
    const metadata = {
      profile,
      types: [],
      members: [
        { id: "u1", name: "用户一" },
        { id: "u2", name: "用户二" },
      ],
      sprints: [],
      versions: [],
      fields: [
        { id, type, format: "multiUser", required: false, options: [] },
        { id: "verifier", type: "Role", format: "user", options: [] },
        {
          id: "other-users",
          type: "SystemCustomField",
          format: "multiUser",
          options: [],
        },
      ],
    } as unknown as Metadata;
    const defaults = initializeSelectionDefaults(draft, metadata);
    expect(defaults.customFields[id]).toBeUndefined();
    expect(defaults.customFields.verifier).toBe("u1");
    expect(defaults.customFields["other-users"]).toEqual(["u1"]);
    const memory = { customFields: { [id]: ["u2"], verifier: "u2" } };
    expect(
      applySelectionMemory(draft, memory, metadata).customFields[id],
    ).toBeUndefined();
    expect(
      applySelectionMemory(draft, memory, metadata).customFields.verifier,
    ).toBe("u2");
    const selected = { ...draft, customFields: { [id]: ["u1"] } };
    expect(
      applySelectionMemory(selected, memory, metadata).customFields[id],
    ).toEqual(["u1"]);
    const cleared = { ...draft, customFields: { [id]: [] } };
    expect(
      applySelectionMemory(cleared, memory, metadata).customFields[id],
    ).toEqual([]);
    expect(draft.customFields).toEqual({});
  });

  it("新草稿把真实首选项写入数据，保存的失效值不被静默替换", () => {
    const draft = {
      workitemTypeId: "",
      assignedTo: "",
      sprint: "",
      versions: [],
      customFields: { priority: "", labels: "deleted" },
    } as unknown as Draft;
    const metadata = {
      types: [{ id: "type-1", name: "Bug" }],
      members: [{ id: "u1", name: "用户" }],
      sprints: [{ id: "s1", name: "迭代" }],
      versions: [{ id: "v1", name: "版本" }],
      fields: [
        {
          id: "priority",
          name: "优先级",
          format: "list",
          options: [{ id: "p1", name: "高" }],
        },
        {
          id: "labels",
          name: "标签",
          format: "tag",
          options: [{ id: "l1", name: "标签" }],
        },
        {
          id: "mappedLabel",
          name: "映射标签",
          format: "tag",
          options: [{ id: "l2", name: "映射标签" }],
        },
      ],
    } as unknown as Metadata;
    const next = initializeSelectionDefaults(draft, metadata, {
      fieldMappings: { mappedLabel: "labels" },
    } as unknown as import("../src/shared/domain").Profile);
    expect(next.workitemTypeId).toBe("type-1");
    expect(next.assignedTo).toBe("u1");
    expect(next.versions).toEqual(["v1"]);
    expect(next).not.toHaveProperty("cloud");
    expect(next.customFields.priority).toBe("p1");
    expect(next.customFields.labels).toBe("deleted");
    expect(next.customFields.mappedLabel).toBeUndefined();
  });

  it("权限不足保留主界面，确认令牌失效才返回登录", () => {
    expect(remoteAuthReaction("403 permission denied")).toBe("stay");
    expect(remoteAuthReaction("network timeout")).toBe("stay");
    expect(remoteAuthReaction("401 unauthorized")).toBe("logout");
  });

  it("项目设置默认停用新项目并保留已保存配置，只提示非空包名冲突", () => {
    const projects = [
      { id: "p1", name: "项目一" },
      { id: "p2", name: "项目二" },
    ];
    const settings = normalizeProjectSettings(projects, {
      entries: [
        { projectId: "p1", enabled: false, packageName: "com.expected" },
      ],
    });
    expect(settings.entries[1].enabled).toBe(false);
    expect(enabledProjects(projects, settings)).toEqual([]);
    const saved = normalizeProjectSettings(projects, {
      entries: [
        { projectId: "p1", enabled: false, packageName: "com.expected" },
        { projectId: "p2", enabled: true, packageName: "com.saved" },
      ],
    });
    expect(saved.entries[1]).toEqual({
      projectId: "p2",
      enabled: true,
      packageName: "com.saved",
    });
    expect(configuredProjectPackage(saved, "p2")).toBe("com.saved");
    expect(configuredProjectPackage(saved, "p1")).toBe("");
    expect(packageMismatch(projects[0], settings, "com.actual")).toContain(
      "com.expected",
    );
    expect(packageMismatch(projects[0], settings, "")).toBe("");
    expect(packageMismatch(projects[1], settings, "com.actual")).toBe("");
  });

  it("项目配置只按已保存的勾选状态排序，保存后才更新顺序", () => {
    const projects = [
      { id: "p1", name: "项目一" },
      { id: "p2", name: "项目二" },
      { id: "p3", name: "项目三" },
      { id: "p4", name: "项目四" },
    ];
    const savedSettings = {
      entries: [
        { projectId: "p1", enabled: false, packageName: "" },
        { projectId: "p2", enabled: true, packageName: "" },
        { projectId: "p3", enabled: false, packageName: "" },
        { projectId: "p4", enabled: true, packageName: "" },
      ],
    };
    const editingSettings = {
      entries: savedSettings.entries.map((entry) => ({
        ...entry,
        enabled:
          entry.projectId === "p1"
            ? true
            : entry.projectId === "p2"
              ? false
              : entry.enabled,
      })),
    };

    expect(
      projectsWithEnabledFirst(projects, savedSettings).map((item) => item.id),
    ).toEqual(["p2", "p4", "p1", "p3"]);
    expect(
      projectsWithEnabledFirst(projects, editingSettings).map(
        (item) => item.id,
      ),
    ).toEqual(["p1", "p4", "p2", "p3"]);
    expect(projects.map((item) => item.id)).toEqual(["p1", "p2", "p3", "p4"]);
  });
});

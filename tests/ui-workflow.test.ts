import { describe, expect, it } from "vitest";
import {
  enqueueSelectionWrite,
  applySelectionMemory,
  rememberedValuesCompatible,
  profileConfigured,
  remoteAuthReaction,
  tabForAuth,
  initialWorkspace,
} from "../src/renderer/session";
import type { Draft, Metadata } from "../src/shared/domain";
import { nativeTarget } from "../src/shared/validation";

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
      business: "旧业务",
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
        business: "新业务",
        workitemTypeId: "type-2",
        assignedTo: "u2",
        versions: ["deleted"],
        customFields: { priority: "P1", untouched: "replace" },
      },
      metadata,
    );
    expect(next.kind).toBe("UI");
    expect(next.business).toBe("新业务");
    expect(next.assignedTo).toBe("u2");
    expect(next.versions).toEqual([]);
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

  it("权限不足保留主界面，确认令牌失效才返回登录", () => {
    expect(remoteAuthReaction("403 permission denied")).toBe("stay");
    expect(remoteAuthReaction("network timeout")).toBe("stay");
    expect(remoteAuthReaction("401 unauthorized")).toBe("logout");
  });
});

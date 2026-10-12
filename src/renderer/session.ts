import type {
  Draft,
  Field,
  Metadata,
  Profile,
  Session,
  Submission,
} from "../shared/domain";
import type { AuthResult, SelectionMemory } from "../shared/workspace";
import {
  excludedField,
  hiddenOptionalField,
} from "../shared/submission-fields";
import { nativeTarget } from "../shared/validation";

export function defaultUnchecked(
  field: Field,
  profile?: Profile | null,
): boolean {
  const target = profile
    ? nativeTarget(field, profile)
    : ["NativeField", "Role"].includes(field.type)
      ? field.id
      : undefined;
  return target === "participants" || target === "trackers";
}

export type AppTab = "auth" | "new" | "history";

export function profileConfigured(profile: Profile): boolean {
  return ![profile.organizationId, profile.projectId, profile.name].some(
    (value) => !value || value === "unconfigured" || value === "请选择项目",
  );
}

export function remoteAuthReaction(error: unknown): "logout" | "stay" {
  return /(^|\D)401(\D|$)|unauthorized|令牌失效|授权失效|token.*(expired|invalid)|过期/i.test(
    String(error),
  )
    ? "logout"
    : "stay";
}

export function initialWorkspace() {
  return {
    profile: {
      name: "请选择项目",
      edition: "central",
      organizationId: "unconfigured",
      projectId: "unconfigured",
      requiredFieldIds: [],
      fieldMappings: {},
      enableVersions: true,
      enableSprints: true,
      attachmentWarningMb: 50,
    } as Profile,
    authorized: false,
    user: null,
    metadata: null as Metadata | null,
    adbPath: null as string | null,
    drafts: [] as Draft[],
    submissions: [] as Submission[],
    sessions: [] as Session[],
  };
}

export function tabForAuth(result: Pick<AuthResult, "status">): AppTab {
  return result.status === "valid" ? "new" : "auth";
}

export function rememberedValuesCompatible(
  remembered: string | string[] | undefined,
  allowed: readonly string[] | undefined,
): boolean {
  if (
    remembered == null ||
    (Array.isArray(remembered) && remembered.length === 0)
  ) {
    return false;
  }
  if (!allowed) return true;
  const values = Array.isArray(remembered) ? remembered : [remembered];
  return values.every((value) => allowed.includes(value));
}

export function enqueueSelectionWrite(
  queue: Promise<unknown>,
  write: () => Promise<unknown>,
  onError: (error: unknown) => void,
): Promise<unknown> {
  return queue
    .catch(() => undefined)
    .then(write)
    .catch((error) => {
      onError(error);
    });
}

export function applySelectionMemory(
  draft: Draft,
  memory: Partial<SelectionMemory>,
  metadata: Metadata | null,
  profile?: Profile | null,
): Draft {
  const next = structuredClone(draft);
  const options = {
    workitemTypeId: metadata?.types.map((item) => item.id),
    assignedTo: metadata?.members.map((item) => item.id),
    sprint: metadata?.sprints.map((item) => item.id),
    versions: metadata?.versions.map((item) => item.id),
  } as const;
  const scalarKeys = [
    "workitemTypeId",
    "assignedTo",
    "sprint",
    "kind",
  ] as const;
  for (const key of scalarKeys) {
    const value = memory[key];
    if (typeof value !== "string" || !value) continue;
    const allowed =
      key in options ? options[key as keyof typeof options] : undefined;
    if (
      allowed?.length &&
      allowed.includes(value) &&
      (key !== "kind" ||
        [
          "功能",
          "UI",
          "多语言",
          "动态过程",
          "崩溃",
          "ANR",
          "性能",
          "网络",
          "数据",
        ].includes(value))
    ) {
      next[key] = value as never;
    }
    if (
      key === "kind" &&
      [
        "功能",
        "UI",
        "多语言",
        "动态过程",
        "崩溃",
        "ANR",
        "性能",
        "网络",
        "数据",
      ].includes(value)
    )
      next.kind = value as Draft["kind"];
  }
  if (Array.isArray(memory.versions) && options.versions?.length) {
    next.versions = memory.versions.filter((value) =>
      options.versions!.includes(value),
    );
  }
  if (
    metadata &&
    memory.customFields &&
    typeof memory.customFields === "object"
  ) {
    for (const field of metadata.fields) {
      if (defaultUnchecked(field, profile ?? metadata.profile)) continue;
      if (
        profile &&
        (excludedField(field, profile) || hiddenOptionalField(field, profile))
      )
        continue;
      const value = memory.customFields[field.id];
      if (typeof value !== "string" && !Array.isArray(value)) continue;
      const dropdown = [
        "list",
        "multiList",
        "user",
        "multiUser",
        "sprint",
        "version",
        "tag",
      ].includes(field.format);
      if (!dropdown) continue;
      const source = ["user", "multiUser"].includes(field.format)
        ? metadata.members
        : field.format === "sprint"
          ? metadata.sprints
          : field.format === "version"
            ? metadata.versions
            : field.options;
      const allowed = source.map((item) => item.id);
      if (!allowed.length) continue;
      const values = Array.isArray(value)
        ? value.filter((item) => allowed.includes(item))
        : value;
      if (Array.isArray(value) && !values.length) continue;
      if (!Array.isArray(value) && !allowed.includes(value)) continue;
      next.customFields[field.id] = Array.isArray(values)
        ? [...values]
        : values;
    }
  }
  return initializeSelectionDefaults(next, metadata, profile);
}

/** Fill only genuinely absent selections for a new draft. Saved invalid ids stay visible. */
export function initializeSelectionDefaults(
  draft: Draft,
  metadata: Metadata | null,
  profile?: Profile | null,
): Draft {
  const next = structuredClone(draft);
  const first = (items?: { id: string }[]) => items?.[0]?.id ?? "";
  if (!next.workitemTypeId && metadata?.types?.length)
    next.workitemTypeId = first(metadata.types);
  if (!next.assignedTo && metadata?.members?.length)
    next.assignedTo = first(metadata.members);
  if (!next.sprint && metadata?.sprints?.length)
    next.sprint = first(metadata.sprints);
  if (!next.versions.length && metadata?.versions?.length)
    next.versions = [first(metadata.versions)];
  if (metadata) {
    for (const field of metadata.fields) {
      if (defaultUnchecked(field, profile ?? metadata.profile)) continue;
      if (
        profile &&
        (excludedField(field, profile) || hiddenOptionalField(field, profile))
      )
        continue;
      const format = field.format;
      const dropdown = [
        "list",
        "multiList",
        "user",
        "multiUser",
        "sprint",
        "version",
        "tag",
      ].includes(format);
      if (!dropdown) continue;
      const source = ["user", "multiUser"].includes(format)
        ? metadata.members
        : format === "sprint"
          ? metadata.sprints
          : format === "version"
            ? metadata.versions
            : field.options;
      const current = next.customFields[field.id];
      const absent =
        current == null ||
        current === "" ||
        (Array.isArray(current) && !current.length);
      if (absent && source.length)
        next.customFields[field.id] = [
          "multiList",
          "multiUser",
          "version",
          "tag",
        ].includes(format)
          ? [source[0].id]
          : source[0].id;
    }
  }
  return next;
}

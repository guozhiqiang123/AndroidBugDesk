import type { Draft, Metadata, Profile } from "../shared/domain";
import type { AuthResult, SelectionMemory } from "../shared/workspace";

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

export async function workspaceDriver(
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>,
): Promise<{ screen: AppTab; configured: boolean; metadataLoaded: boolean }> {
  const auth = (await invoke("check_auth")) as AuthResult;
  if (auth.status !== "valid")
    return { screen: "auth", configured: false, metadataLoaded: false };
  const bootstrap = (await invoke("bootstrap")) as { profile: Profile };
  try {
    const organizations = (await invoke("list_organizations")) as {
      id: string;
    }[];
    const profile = bootstrap.profile;
    const orgVisible = organizations.some(
      (item) => item.id === profile.organizationId,
    );
    if (!orgVisible)
      return { screen: "new", configured: false, metadataLoaded: false };
    const projects = (await invoke("list_projects", {
      organizationId: profile.organizationId,
    })) as { id: string }[];
    const projectVisible = projects.some(
      (item) => item.id === profile.projectId,
    );
    if (!projectVisible)
      return { screen: "new", configured: false, metadataLoaded: false };
    await invoke("load_metadata", { typeId: "" });
    await invoke("new_draft");
    await invoke("load_selection_memory", { typeId: "" });
    return { screen: "new", configured: true, metadataLoaded: true };
  } catch (error) {
    if (remoteAuthReaction(error) === "logout")
      return { screen: "auth", configured: false, metadataLoaded: false };
    return { screen: "new", configured: false, metadataLoaded: false };
  }
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
    "business",
  ] as const;
  for (const key of scalarKeys) {
    const value = memory[key];
    if (typeof value !== "string" || !value) continue;
    const allowed =
      key in options ? options[key as keyof typeof options] : undefined;
    const businessAllowed = profile?.businessProfiles;
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
        ].includes(value)) &&
      (key !== "business" || businessAllowed?.includes(value))
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
    if (
      key === "business" &&
      (!businessAllowed || businessAllowed.includes(value))
    )
      next.business = value;
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
  return next;
}

import type { Option } from "../shared/domain";
export type { ProjectSetting, ProjectSettings } from "../shared/workspace";
import type { ProjectSettings } from "../shared/workspace";

export function normalizeProjectSettings(
  projects: Option[],
  settings: ProjectSettings | null | undefined,
): ProjectSettings {
  const byId = new Map(
    (settings?.entries ?? []).map((item) => [item.projectId, item]),
  );
  return {
    entries: projects.map((project) => ({
      projectId: project.id,
      enabled: byId.get(project.id)?.enabled ?? false,
      packageName: byId.get(project.id)?.packageName ?? "",
    })),
  };
}

export function enabledProjects(
  projects: Option[],
  settings: ProjectSettings | null | undefined,
): Option[] {
  const enabled = new Set(
    (settings?.entries ?? [])
      .filter((item) => item.enabled)
      .map((item) => item.projectId),
  );
  return projects.filter((project) => enabled.has(project.id));
}

export function configuredProjectPackage(
  settings: ProjectSettings | null | undefined,
  projectId: string,
): string {
  const entry = settings?.entries.find(
    (item) => item.projectId === projectId && item.enabled,
  );
  return entry?.packageName.trim() ?? "";
}

export function packageMismatch(
  project: Option | undefined,
  settings: ProjectSettings | null | undefined,
  draftPackageName: string,
): string {
  if (!project || !draftPackageName) return "";
  const expected =
    settings?.entries
      .find((item) => item.projectId === project.id)
      ?.packageName.trim() ?? "";
  return expected && expected !== draftPackageName
    ? `项目「${project.name}」配置包名 ${expected} 与当前草稿包名 ${draftPackageName} 不一致。`
    : "";
}

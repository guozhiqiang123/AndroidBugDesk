import type { Draft, Metadata, Option, Profile, Submission } from "./domain";
import { nativeTarget } from "./validation";

export const historyFields = [
  ["kind", "问题类型"],
  ["workitemTypeId", "云效 Bug 类型"],
  ["assignedTo", "负责人"],
  ["sprint", "迭代"],
  ["verifier", "验证者"],
  ["priority", "优先级"],
  ["severity", "严重程度"],
  ["participants", "参与者"],
  ["trackers", "抄送"],
] as const;
export type HistoryFilterKey = (typeof historyFields)[number][0];
export type HistoryFilters = Record<HistoryFilterKey, string[]> & {
  keyword: string;
};
export const historyKinds = [
  "功能",
  "UI",
  "多语言",
  "动态过程",
  "崩溃",
  "ANR",
  "性能",
  "网络",
  "数据",
];
export function emptyHistoryFilters(): HistoryFilters {
  return Object.fromEntries([
    ...historyFields.map(([key]) => [key, []]),
    ["keyword", ""],
  ]) as HistoryFilters;
}

export function historyFieldIds(
  key: HistoryFilterKey,
  metadata: Metadata | null,
): string[] {
  const names: Record<string, string[]> = {
    priority: ["优先级"],
    severity: ["严重程度", "严重等级", "严重性"],
  };
  return [
    ...new Set([
      key,
      ...(metadata?.fields
        .filter(
          (field) =>
            nativeTarget(field, metadata.profile) === key ||
            names[key]?.includes(field.name.trim()),
        )
        .map((field) => field.id) ?? []),
    ]),
  ];
}
function values(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((v): v is string => typeof v === "string")
    : typeof value === "string" && value
      ? [value]
      : [];
}
export function draftHistoryValues(
  draft: Draft,
  key: HistoryFilterKey,
  metadata: Metadata | null,
): string[] {
  if (["kind", "workitemTypeId", "assignedTo", "sprint"].includes(key))
    return values(
      draft[key as "kind" | "workitemTypeId" | "assignedTo" | "sprint"],
    );
  return [
    ...new Set(
      historyFieldIds(key, metadata).flatMap((id) =>
        values(draft.customFields[id]),
      ),
    ),
  ];
}
export function localHistoryRows(
  submissions: Submission[],
  drafts: Draft[],
  profile: Profile,
  filters: HistoryFilters,
  metadata: Metadata | null,
): Submission[] {
  const byId = new Map(drafts.map((draft) => [draft.id, draft]));
  const keyword = filters.keyword.trim().toLocaleLowerCase();
  return submissions
    .filter((submission) => {
      if (
        submission.profile.organizationId !== profile.organizationId ||
        submission.profile.projectId !== profile.projectId
      )
        return false;
      const draft = byId.get(submission.draftId);
      if (
        keyword &&
        ![draft?.title, submission.identifier, submission.workitemId].some(
          (v) => v?.toLocaleLowerCase().includes(keyword),
        )
      )
        return false;
      return historyFields.every(
        ([key]) =>
          !filters[key].length ||
          (draft &&
            draftHistoryValues(draft, key, metadata).some((v) =>
              filters[key].includes(v),
            )),
      );
    })
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}
export function historyOptions(
  key: HistoryFilterKey,
  metadata: Metadata | null,
  localDrafts: Draft[] = [],
): Option[] {
  let options: Option[] = [];
  if (key === "kind")
    options = [...historyKinds, "未记录"].map((id) => ({ id, name: id }));
  else if (key === "workitemTypeId") options = metadata?.types ?? [];
  else if (key === "sprint") options = metadata?.sprints ?? [];
  else if (["assignedTo", "verifier", "participants", "trackers"].includes(key))
    options = metadata?.members ?? [];
  else
    options =
      metadata?.fields
        .filter((field) => historyFieldIds(key, metadata).includes(field.id))
        .flatMap((field) => field.options) ?? [];
  const unique = new Map(options.map((option) => [option.id, option]));
  for (const draft of localDrafts)
    for (const id of draftHistoryValues(draft, key, metadata))
      if (!unique.has(id)) unique.set(id, { id, name: id });
  return [...unique.values()];
}

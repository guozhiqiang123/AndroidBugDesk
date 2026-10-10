import type {
  Draft,
  Metadata,
  Profile,
  ValidationIssue,
  Field,
} from "./domain";
import {
  excludedField,
  excludedFieldId,
  hiddenOptionalField,
} from "./submission-fields";
export const useful = (v: unknown): boolean =>
  typeof v === "string" &&
  !!v.trim() &&
  !/^(未知|待定|待补充|稍后补|无|不清楚|unknown|todo|tbd|n\/a|-+)$/i.test(
    v.trim(),
  );
export const zonedTime = (v: string): boolean =>
  /^\d{4}-\d\d-\d\dT\d\d:\d\d(:\d\d(\.\d+)?)?(Z|[+-]\d\d:\d\d)$/.test(v) &&
  Number.isFinite(Date.parse(v));
export function parseBuildUrl(
  value: string,
): { number: string; job: string } | null {
  try {
    const u = new URL(value);
    if (
      !["http:", "https:"].includes(u.protocol) ||
      u.username ||
      u.password ||
      u.search ||
      u.hash
    )
      return null;
    const match = u.pathname.match(/^(.*(?:\/job\/[^/]+)+)\/([1-9]\d*)\/?$/);
    if (
      !match ||
      /\/(lastBuild|lastSuccessfulBuild|lastStableBuild|lastCompletedBuild)\//i.test(
        u.pathname,
      )
    )
      return null;
    return { job: decodeURIComponent(match[1]), number: match[2] };
  } catch {
    return null;
  }
}
export function nativeTarget(
  field: Field,
  profile: Profile,
): string | undefined {
  return (
    profile.fieldMappings[field.id] ??
    ((field.type === "NativeField" ||
      (field.type === "Role" &&
        ["verifier", "participants", "trackers"].includes(field.id)) ||
      (field.type === "Application" &&
        ["sprint", "versions", "labels"].includes(field.id))) &&
    [
      "subject",
      "description",
      "assignedTo",
      "sprint",
      "versions",
      "verifier",
      "participants",
      "labels",
      "trackers",
    ].includes(field.id)
      ? field.id
      : undefined)
  );
}
export const supportedFormats = new Set([
  "list",
  "multiList",
  "tag",
  "string",
  "text",
  "int",
  "float",
  "date",
  "dateTime",
  "user",
  "multiUser",
  "sprint",
  "version",
]);
export function validateDraft(
  d: Draft,
  profile: Profile,
  metadata: Metadata | null,
  authorized: boolean,
  online = true,
): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const add = (field: string, message: string) =>
    issues.push({ field, message });
  const req = (field: string, value: unknown, label: string) => {
    if (!useful(value)) add(field, `请补充${label}`);
  };
  if (online && !authorized) add("auth", "请填写个人 PAT 并读取项目配置");
  if (
    online &&
    (!metadata ||
      metadata.profile.projectId !== profile.projectId ||
      metadata.profile.organizationId !== profile.organizationId ||
      metadata.typeId !== d.workitemTypeId)
  )
    add("project", "请重新读取当前项目和缺陷类型配置");
  req("title", d.title, "问题标题");
  req("app.packageName", d.app.packageName, "当前项目的 Android 包名配置");
  req("preconditions", d.preconditions, "复现条件");
  const steps = d.steps
    .split("\n")
    .map((s) => s.replace(/^\s*\d+[.、)]\s*/, "").trim())
    .filter(Boolean);
  if (
    !steps.length ||
    steps.some(
      (s) => s.length < 6 || /^(打开看一下|打开看看|查看一下)$/.test(s),
    )
  )
    add("steps", "请逐行填写可执行的复现步骤");
  req("expected", d.expected, "期望表现");
  req("actual", d.actual, "问题表现");
  if (!d.frequency.firstDiscovery) {
    const { attempts, occurrences } = d.frequency;
    if (
      !/^\d+$/.test(attempts) ||
      !/^\d+$/.test(occurrences) ||
      Number(attempts) < 1 ||
      Number(occurrences) < 1 ||
      Number(occurrences) > Number(attempts)
    )
      add("frequency", "发生次数应为 1 至尝试次数之间的整数");
  }
  for (const a of d.attachments) {
    if (a.role === "log") {
      if (!a.size || !useful(a.source))
        add(
          `attachment.${a.id}.confirmed`,
          "所选日志为空或来源缺失，请重新导入",
        );
      if (!zonedTime(a.coverageStart))
        add(
          `attachment.${a.id}.coverageStart`,
          "请填写日志的真实覆盖起点（含时区）",
        );
      if (
        !zonedTime(a.coverageEnd) ||
        Date.parse(a.coverageEnd) < Date.parse(a.coverageStart)
      )
        add(
          `attachment.${a.id}.coverageEnd`,
          "请填写日志的真实覆盖终点，且不得早于起点（含时区）",
        );
    }
    const paired = d.attachments.filter(
      (r) => r.role === "report" && r.derivedFrom === a.id,
    );
    const window = a.source.startsWith("ADB 问题日志窗口派生副本");
    const redacted = a.source.startsWith("文本派生副本");
    const rangeRequired =
      window ||
      a.source.includes("来源为问题日志窗口") ||
      (redacted &&
        d.attachments.some(
          (parent) =>
            parent.id === a.derivedFrom &&
            parent.source.startsWith("ADB 问题日志窗口派生副本"),
        ));
    if (
      a.role === "log" &&
      (window || redacted || paired.length) &&
      ((rangeRequired &&
        !paired.some((r) => r.source === "日志窗口范围与缺口记录")) ||
        (redacted &&
          !paired.some((r) => r.source === "文本替换记录（请核对副本）")))
    )
      add("attachments", "派生副本必须携带全部来源回执");
    if (
      a.role === "report" &&
      a.derivedFrom &&
      !d.attachments.some((parent) => parent.id === a.derivedFrom)
    )
      add("attachments", "来源回执必须保留关联副本");
  }
  if (d.kind === "网络")
    for (const k of ["operation", "type", "error", "collectionStatus"] as const)
      req(`network.${k}`, d.network[k], "网络操作、类型、可观察错误及采集状态");
  if (d.kind === "数据")
    for (const k of ["type", "count", "source"] as const)
      req(`data.${k}`, d.data[k], "数据类型、数量及来源");
  if (online && metadata) {
    if (!metadata.members.some((m) => m.id === d.assignedTo))
      add("assignedTo", "请选择当前项目真实成员");
    if (!d.sprint && !d.versions.length)
      add("scope", "至少选择一个真实版本或迭代");
    if (d.sprint && !metadata.sprints.some((s) => s.id === d.sprint))
      add("sprint", "迭代 ID 已失效");
    if (d.versions.some((id) => !metadata.versions.some((v) => v.id === id)))
      add("versions", "版本 ID 已失效");
    for (const field of metadata.fields) {
      const target = nativeTarget(field, profile);
      const required =
        field.required || profile.requiredFieldIds.includes(field.id);
      if (excludedField(field, profile)) {
        if (required)
          add(
            `customFields.${field.id}`,
            `云效项目将已移除字段「${field.name}」设为必填，请调整项目配置后提交`,
          );
        continue;
      }
      if (hiddenOptionalField(field, profile)) continue;
      let value: string | string[] | undefined =
        target === "subject"
          ? d.title
          : target === "description"
            ? d.actual
            : target === "assignedTo"
              ? d.assignedTo
              : target === "sprint"
                ? d.sprint
                : target === "versions"
                  ? d.versions
                  : d.customFields[field.id];
      const present = Array.isArray(value)
        ? value.length > 0 && value.every(useful)
        : useful(value);
      if (
        (required || present) &&
        (!supportedFormats.has(field.format) ||
          (!target &&
            !["CustomField", "SystemCustomField"].includes(field.type)))
      ) {
        add(
          `customFields.${field.id}`,
          `字段「${field.name}」的格式或映射未支持，请完善项目配置`,
        );
        continue;
      }
      if (required && !present)
        add(`customFields.${field.id}`, `请补充云效必填字段「${field.name}」`);
      if (
        !present ||
        (target &&
          [
            "subject",
            "description",
            "assignedTo",
            "sprint",
            "versions",
          ].includes(target))
      )
        continue;
      const values = Array.isArray(value) ? value : [value!];
      const options =
        field.format.includes("User") || field.format === "user"
          ? metadata.members
          : field.format === "sprint"
            ? metadata.sprints
            : field.format === "version"
              ? metadata.versions
              : field.options;
      if (
        [
          "list",
          "multiList",
          "tag",
          "user",
          "multiUser",
          "sprint",
          "version",
        ].includes(field.format) &&
        values.some((v) => !options.some((o) => o.id === v))
      )
        add(`customFields.${field.id}`, `「${field.name}」选项已失效`);
      if (
        field.format === "int" &&
        values.some(
          (v) => !/^-?\d+$/.test(v) || !Number.isSafeInteger(Number(v)),
        )
      )
        add(`customFields.${field.id}`, `「${field.name}」必须为整数`);
      if (
        field.format === "float" &&
        values.some((v) => !Number.isFinite(Number(v)))
      )
        add(`customFields.${field.id}`, `「${field.name}」必须为数值`);
      if (
        field.format === "date" &&
        values.some(
          (v) =>
            !/^\d{4}-\d\d-\d\d$/.test(v) || !Number.isFinite(Date.parse(v)),
        )
      )
        add(`customFields.${field.id}`, `「${field.name}」日期无效`);
      if (field.format === "dateTime" && values.some((v) => !zonedTime(v)))
        add(`customFields.${field.id}`, `「${field.name}」时间缺少时区`);
    }
    for (const [id, v] of Object.entries(d.customFields))
      if (
        (Array.isArray(v) ? v.length > 0 : useful(v)) &&
        !excludedFieldId(id, profile) &&
        !metadata.fields.some((f) => f.id === id)
      )
        add(`customFields.${id}`, `字段 ${id} 已不在当前配置中，请核对后移除`);
    for (const id of profile.requiredFieldIds)
      if (!metadata.fields.some((f) => f.id === id))
        add("project", `项目必填字段 ${id} 未出现在云效元数据中`);
  }
  if (online && !d.previewConfirmed)
    add("previewConfirmed", "请预览并确认项目、描述和附件");
  return [...new Map(issues.map((i) => [i.field + i.message, i])).values()];
}

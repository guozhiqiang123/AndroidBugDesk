import type {
  Draft,
  Metadata,
  Profile,
  ValidationIssue,
  Field,
} from "./domain";
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
  if (!profile.businessProfiles.includes(d.business))
    add("business", "请选择配置中的业务方向");
  req("title", d.title, "具体问题标题");
  if (d.title.trim().length < 5)
    add("title", "标题至少 5 个字符，并说明具体行为");
  if (!parseBuildUrl(d.buildUrl))
    add("buildUrl", "请填写 Jenkins 固定构建号 URL");
  if (d.buildStatus === "conflict")
    add("buildStatus", "构建身份存在冲突，需重新选择实际构建");
  if (d.buildStatus === "unconfirmed")
    add("buildStatus", "请确认固定构建与当前安装包的关系");
  if (
    d.buildManifest &&
    (["packageName", "versionName", "versionCode"] as const).some(
      (k) => d.buildManifest![k] !== d.app[k],
    )
  )
    add("buildStatus", "构建清单与设备应用身份不一致");
  for (const k of [
    "packageName",
    "versionName",
    "channel",
    "environment",
    "obfuscation",
    "hardening",
  ] as const)
    req(
      `app.${k}`,
      d.app[k],
      {
        packageName: "包名",
        versionName: "应用版本",
        channel: "渠道",
        environment: "测试环境",
        obfuscation: "混淆状态",
        hardening: "加固状态",
      }[k],
    );
  if (!/^[A-Za-z][\w]*(\.[A-Za-z][\w]*)+$/.test(d.app.packageName))
    add("app.packageName", "包名格式不正确");
  if (
    !/^\d+$/.test(d.app.versionCode) ||
    !Number.isSafeInteger(Number(d.app.versionCode))
  )
    add("app.versionCode", "versionCode 必须是非负整数（允许 0）");
  for (const k of [
    "model",
    "androidVersion",
    "totalMemKb",
    "availableMemKb",
    "resolution",
    "density",
  ] as const) {
    req(
      `device.${k}`,
      d.device[k],
      {
        model: "设备型号",
        androidVersion: "Android 版本",
        totalMemKb: "总内存（kB）",
        availableMemKb: "当前可用内存（kB）",
        resolution: "像素分辨率",
        density: "屏幕密度（dpi）",
      }[k],
    );
    if (
      useful(d.device[k]) &&
      d.sources[`device.${k}`]?.status !== "collected" &&
      !useful(d.manualNote)
    )
      add("manualNote", "人工补充设备信息时需注明来源");
  }
  for (const k of ["totalMemKb", "availableMemKb", "density"] as const)
    if (
      !/^\d+(\.\d+)?$/.test(d.device[k]) ||
      Number(d.device[k]) < 0 ||
      (k !== "availableMemKb" && Number(d.device[k]) === 0)
    )
      add(`device.${k}`, "请输入有效数值和界面指定单位");
  if (Number(d.device.availableMemKb) > Number(d.device.totalMemKb))
    add("device.availableMemKb", "可用内存不能大于总内存");
  if (!/^\d+[x×]\d+$/.test(d.device.resolution))
    add("device.resolution", "分辨率格式为 1080x2400");
  if (!zonedTime(d.event.occurredAt))
    add("event.occurredAt", "问题时间必须包含时区");
  if (
    d.event.source === "manual" ||
    (d.event.originalAt && d.event.originalAt !== d.event.occurredAt)
  )
    req("event.sourceNote", d.event.sourceNote, "人工事件时间来源或修改依据");
  if (
    d.event.lateCapture &&
    !d.attachments.some(
      (a) => a.confirmed && a.size > 0 && a.source.startsWith("导入"),
    )
  )
    add("event.lateCapture", "事后补采需导入已有证据或重新复现并自动标记");
  req("preconditions", d.preconditions, "前置条件");
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
  req("expected", d.expected, "预期结果");
  req("actual", d.actual, "实际结果");
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
  if (
    !d.attachments.some(
      (a) =>
        a.role === "log" &&
        a.size > 0 &&
        a.confirmed &&
        useful(a.source) &&
        zonedTime(a.coverageStart) &&
        zonedTime(a.coverageEnd) &&
        Date.parse(a.coverageEnd) >= Date.parse(a.coverageStart),
    )
  )
    add("attachments", "请添加并确认非空日志及真实覆盖起止时间");
  if (
    ["UI", "多语言", "动态过程"].includes(d.kind) &&
    !d.attachments.some(
      (a) => ["image", "video"].includes(a.role) && a.confirmed && a.size > 0,
    )
  )
    add("attachments", "显示问题需要截图或视频");
  if (
    d.kind === "动态过程" &&
    !d.attachments.some((a) => a.role === "video" && a.confirmed) &&
    d.attachments.filter((a) => a.role === "image" && a.confirmed).length < 2
  )
    add("attachments", "动态过程需要视频或连续多张截图");
  if (d.kind === "网络")
    for (const k of ["operation", "type", "error", "collectionStatus"] as const)
      req(`network.${k}`, d.network[k], "网络操作、类型、可观察错误及采集状态");
  if (d.kind === "数据")
    for (const k of ["type", "count", "source"] as const)
      req(`data.${k}`, d.data[k], "数据类型、数量及来源");
  if (d.cloud.applicability === "unset")
    add("cloud.applicability", "请声明云控适用性");
  if (d.cloud.applicability === "no")
    req("cloud.reason", d.cloud.reason, "该功能无云控的依据");
  if (d.cloud.applicability === "yes") {
    for (const k of ["name", "effectiveValue", "source", "override"] as const)
      req(`cloud.${k}`, d.cloud[k], "设备云控有效值、来源和测试覆盖声明");
    if (
      !["应用调试面板", "应用导出", "相关运行日志", "用户可观察行为"].includes(
        d.cloud.source,
      )
    )
      add("cloud.source", "请选择设备生效证据来源");
    if (!zonedTime(d.cloud.observedAt))
      add("cloud.observedAt", "云控观察时间必须包含时区");
  }
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
        !metadata.fields.some((f) => f.id === id)
      )
        add(`customFields.${id}`, `字段 ${id} 已不在当前配置中，请核对后移除`);
    for (const id of profile.requiredFieldIds)
      if (!metadata.fields.some((f) => f.id === id))
        add("project", `项目必填字段 ${id} 未出现在云效元数据中`);
  }
  if (online && !d.previewConfirmed)
    add("previewConfirmed", "请预览并确认项目、构建、描述和附件");
  return [...new Map(issues.map((i) => [i.field + i.message, i])).values()];
}

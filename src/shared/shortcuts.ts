export const shortcutDefinitions = [
  {
    id: "newIssue",
    title: "新建问题",
    defaultBinding: "Primary+n",
  },
  {
    id: "environmentSnapshot",
    title: "设备环境快照",
    defaultBinding: "Primary+e",
  },
  {
    id: "postCapture",
    title: "补采证据",
    defaultBinding: "Primary+g",
  },
  {
    id: "importAttachments",
    title: "导入附件",
    defaultBinding: "Primary+o",
  },
  {
    id: "saveDraft",
    title: "保存草稿",
    defaultBinding: "Primary+s",
  },
  {
    id: "exportEvidence",
    title: "导出本地证据",
    defaultBinding: "Primary+d",
  },
  {
    id: "submissionPreview",
    title: "进入提交预览",
    defaultBinding: "Primary+Enter",
  },
] as const;

export type ShortcutAction = (typeof shortcutDefinitions)[number]["id"];
export type ShortcutPlatform = "macos" | "windows" | "linux";
export type ShortcutBindings = Record<ShortcutAction, string>;
export interface ShortcutSettings {
  version: 1;
  overrides: Partial<Record<ShortcutAction, string>>;
}
export interface ShortcutPresentation {
  action: ShortcutAction;
  display: string;
  aria: string;
}

export const defaultShortcutSettings = (): ShortcutSettings => ({
  version: 1,
  overrides: {},
});

export const shortcutActionIds = new Set<ShortcutAction>(
  shortcutDefinitions.map((item) => item.id),
);

export function shortcutTitle(action: ShortcutAction) {
  return shortcutDefinitions.find((item) => item.id === action)!.title;
}

export function resolveShortcutBindings(
  settings?: ShortcutSettings | null,
): ShortcutBindings {
  return Object.fromEntries(
    shortcutDefinitions.map((item) => [
      item.id,
      settings?.overrides[item.id] ?? item.defaultBinding,
    ]),
  ) as ShortcutBindings;
}

export function shortcutPlatform(
  identity: Pick<Navigator, "platform" | "userAgent"> = navigator,
): ShortcutPlatform {
  const value = `${identity.platform} ${identity.userAgent}`.toLowerCase();
  if (value.includes("mac")) return "macos";
  if (value.includes("win")) return "windows";
  return "linux";
}

const rejectedEventKeys = new Set([
  "",
  "Dead",
  "Process",
  "Unidentified",
  "AltGraph",
]);

export function shortcutFromKeyboardEvent(
  event: Pick<
    KeyboardEvent,
    | "key"
    | "ctrlKey"
    | "altKey"
    | "shiftKey"
    | "metaKey"
    | "isComposing"
    | "getModifierState"
  >,
  platform: ShortcutPlatform,
): string | null {
  if (
    event.isComposing ||
    event.getModifierState?.("AltGraph") ||
    rejectedEventKeys.has(event.key)
  )
    return null;
  const primary = platform === "macos" ? event.metaKey : event.ctrlKey;
  const unsupportedModifier =
    event.altKey || (platform === "macos" ? event.ctrlKey : event.metaKey);
  if (!primary || unsupportedModifier) return null;
  const key = normalizeShortcutKey(event.key);
  if (!key) return null;
  return `Primary+${event.shiftKey ? "Shift+" : ""}${key}`;
}

export function normalizeShortcutKey(key: string): string | null {
  if (key === "Enter") return key;
  if (/^[a-z0-9]$/iu.test(key)) return key.toLocaleLowerCase();
  return null;
}

export function shortcutValidationError(
  binding: string,
  action: ShortcutAction,
  bindings: ShortcutBindings,
): string {
  const parsed = parseShortcut(binding);
  if (!parsed)
    return "快捷键必须使用 Ctrl/Command，可额外按 Shift，并以字母、数字或 Enter 结束。";
  const protectedEditing = new Set(["a", "c", "v", "x", "y", "z", "f"]);
  if (protectedEditing.has(parsed.key))
    return "该组合属于复制、粘贴、剪切、全选、撤销、重做或查找等常用编辑快捷键，不能占用。";
  if (!parsed.shift && new Set(["b", "i", "u"]).has(parsed.key))
    return "该组合属于常用文本格式快捷键，不能占用。";
  if (new Set(["q", "w", "h", "m"]).has(parsed.key))
    return "该组合可能与系统窗口快捷键冲突，不能占用。";
  const conflict = shortcutDefinitions.find(
    (item) => item.id !== action && bindings[item.id] === binding,
  );
  return conflict ? `与“${conflict.title}”的快捷键冲突。` : "";
}

export function parseShortcut(binding: string) {
  const match = /^Primary\+(Shift\+)?([a-z0-9]|Enter)$/u.exec(binding);
  return match
    ? { shift: Boolean(match[1]), key: match[2]!.toLocaleLowerCase() }
    : null;
}

export function formatShortcut(
  binding: string,
  platform: ShortcutPlatform,
): string {
  const parsed = parseShortcut(binding);
  if (!parsed) return binding;
  const key = parsed.key === "enter" ? "Enter" : parsed.key.toUpperCase();
  return platform === "macos"
    ? `⌘${parsed.shift ? "⇧" : ""}${key}`
    : `Ctrl+${parsed.shift ? "Shift+" : ""}${key}`;
}

export function ariaShortcut(
  binding: string,
  platform: ShortcutPlatform,
): string {
  const parsed = parseShortcut(binding);
  if (!parsed) return "";
  const key = parsed.key === "enter" ? "Enter" : parsed.key.toUpperCase();
  return `${platform === "macos" ? "Meta" : "Control"}+${parsed.shift ? "Shift+" : ""}${key}`;
}

export function shortcutPresentation(
  action: ShortcutAction,
  bindings: ShortcutBindings,
  platform: ShortcutPlatform,
): ShortcutPresentation {
  return {
    action,
    display: formatShortcut(bindings[action], platform),
    aria: ariaShortcut(bindings[action], platform),
  };
}

export function settingsWithBinding(
  settings: ShortcutSettings,
  action: ShortcutAction,
  binding: string,
): ShortcutSettings {
  const defaults = resolveShortcutBindings(defaultShortcutSettings());
  const overrides = { ...settings.overrides };
  if (binding === defaults[action]) delete overrides[action];
  else overrides[action] = binding;
  return { version: 1, overrides };
}

export function settingsWithoutBindingOverride(
  settings: ShortcutSettings,
  action: ShortcutAction,
): ShortcutSettings {
  const overrides = { ...settings.overrides };
  delete overrides[action];
  return { version: 1, overrides };
}

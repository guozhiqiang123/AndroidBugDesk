import { describe, expect, it } from "vitest";
import {
  ariaShortcut,
  defaultShortcutSettings,
  formatShortcut,
  resolveShortcutBindings,
  settingsWithBinding,
  settingsWithoutBindingOverride,
  shortcutDefinitions,
  shortcutFromKeyboardEvent,
  shortcutValidationError,
  type ShortcutBindings,
  type ShortcutPlatform,
} from "../src/shared/shortcuts";

function keyboardEvent(
  key: string,
  platform: ShortcutPlatform,
  options: { shift?: boolean; alt?: boolean; otherPrimary?: boolean } = {},
) {
  return {
    key,
    ctrlKey: platform === "macos" ? Boolean(options.otherPrimary) : true,
    metaKey: platform === "macos" ? true : Boolean(options.otherPrimary),
    shiftKey: Boolean(options.shift),
    altKey: Boolean(options.alt),
    isComposing: false,
    getModifierState: () => false,
  };
}

describe("新建提交快捷键", () => {
  it("默认动作完整且组合不重复", () => {
    const bindings = resolveShortcutBindings(defaultShortcutSettings());
    expect(Object.keys(bindings)).toEqual(
      shortcutDefinitions.map((item) => item.id),
    );
    expect(new Set(Object.values(bindings)).size).toBe(
      shortcutDefinitions.length,
    );
    expect(bindings).toEqual({
      newIssue: "Primary+n",
      environmentSnapshot: "Primary+e",
      postCapture: "Primary+g",
      importAttachments: "Primary+o",
      saveDraft: "Primary+s",
      exportEvidence: "Primary+d",
      submissionPreview: "Primary+Enter",
    });
    expect(
      Object.values(bindings).every((binding) => !binding.includes("Shift")),
    ).toBe(true);
  });

  it("按平台把 Command 或 Control 归一为 Primary", () => {
    expect(
      shortcutFromKeyboardEvent(
        keyboardEvent("N", "macos", { shift: true }),
        "macos",
      ),
    ).toBe("Primary+Shift+n");
    expect(
      shortcutFromKeyboardEvent(
        keyboardEvent("Enter", "windows", { shift: true }),
        "windows",
      ),
    ).toBe("Primary+Shift+Enter");
    expect(
      shortcutFromKeyboardEvent(
        keyboardEvent("n", "linux", { alt: true }),
        "linux",
      ),
    ).toBeNull();
  });

  it("为按钮生成平台提示与无障碍组合", () => {
    expect(formatShortcut("Primary+Shift+n", "macos")).toBe("⌘⇧N");
    expect(formatShortcut("Primary+Shift+n", "windows")).toBe("Ctrl+Shift+N");
    expect(ariaShortcut("Primary+Shift+Enter", "macos")).toBe(
      "Meta+Shift+Enter",
    );
  });

  it("拒绝常见编辑组合、系统组合和动作冲突", () => {
    const bindings = resolveShortcutBindings(defaultShortcutSettings());
    expect(shortcutValidationError("Primary+v", "newIssue", bindings)).toMatch(
      /编辑快捷键/,
    );
    expect(
      shortcutValidationError("Primary+Shift+c", "newIssue", bindings),
    ).toMatch(/编辑快捷键/);
    expect(shortcutValidationError("Primary+q", "newIssue", bindings)).toMatch(
      /系统窗口/,
    );
    expect(
      shortcutValidationError(
        bindings.environmentSnapshot,
        "newIssue",
        bindings,
      ),
    ).toMatch(/设备环境快照/);
  });

  it("只保存偏离默认值的覆盖，并支持单项恢复", () => {
    const defaults = defaultShortcutSettings();
    const customized = settingsWithBinding(
      defaults,
      "newIssue",
      "Primary+Shift+1",
    );
    expect(customized.overrides.newIssue).toBe("Primary+Shift+1");
    const effective = resolveShortcutBindings(customized);
    expect(effective.newIssue).toBe("Primary+Shift+1");
    expect(effective.saveDraft).toBe("Primary+s");
    expect(settingsWithoutBindingOverride(customized, "newIssue")).toEqual(
      defaults,
    );
  });

  it("复制粘贴按键不会匹配任何默认动作", () => {
    const bindings = resolveShortcutBindings(defaultShortcutSettings());
    for (const platform of ["macos", "windows"] as const) {
      for (const key of ["c", "v", "x", "a", "z", "y", "f"]) {
        const binding = shortcutFromKeyboardEvent(
          keyboardEvent(key, platform),
          platform,
        );
        expect(Object.values(bindings)).not.toContain(binding);
      }
    }
  });
});

import React, {
  type CSSProperties,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createRoot } from "react-dom/client";
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import {
  draftSchema,
  type Draft,
  type Profile,
  type Metadata,
  type Submission,
  type Device,
  type Session,
  type Snapshot,
  type Attachment,
  type Option,
  type Source,
} from "../shared/domain";
import type {
  AuthResult,
  SelectionMemory,
  UserIdentity,
  ProjectSettings,
  DraftProjectScope,
  UpdateCheckResult,
  UpdateInstallResult,
  UpdateProgress,
  DeveloperOptions,
} from "../shared/workspace";
import {
  defaultShortcutSettings,
  resolveShortcutBindings,
  settingsWithBinding,
  settingsWithoutBindingOverride,
  shortcutDefinitions,
  shortcutFromKeyboardEvent,
  shortcutPlatform,
  shortcutPresentation,
  shortcutValidationError,
  type ShortcutAction,
  type ShortcutSettings,
} from "../shared/shortcuts";
import appPackage from "../../package.json";
import {
  nativeTarget,
  validateDraft,
  supportedFormats,
} from "../shared/validation";
import { excludedField } from "../shared/submission-fields";
import { enqueueSelectionWrite, tabForAuth } from "./session";
import {
  applySelectionMemory,
  defaultUnchecked,
  profileConfigured,
} from "./session";
import { initialWorkspace, remoteAuthReaction } from "./session";
import "./style.css";
import { UserProfile } from "./user-profile";
import { HistoryView } from "./history-view";
import { EnvironmentSnapshotEntry, PostCaptureEntry } from "./post-capture";
import {
  splitPostCaptureAttachmentDescription,
  type PostCaptureSnapshot,
} from "../shared/post-capture";
import { LoginCard, LoginRestoring } from "./login-card";
import { CustomSelect } from "./custom-select";
import { NotificationToast, useTransientNotice } from "./notification-toast";
import { AppDialog } from "./app-dialog";
import {
  MarkdownEditor,
  MarkdownPreview,
  markdownImageFilePayload,
  type MarkdownImageImportResult,
} from "./markdown-editor";
import {
  attachmentIssueKey,
  groupIssuesByNavigationTarget,
  routeIssues,
} from "./issue-routing";
import {
  configuredProjectPackage,
  enabledProjects,
  normalizeProjectSettings,
  projectsWithEnabledFirst,
} from "./project-settings";
import { ShortcutBadge } from "./shortcut-badge";

interface CaptureStatus {
  id: string;
  draftId: string;
  kind: string;
  status: string;
  error?: string;
  window?: { markedAt: string; dueAt: string };
  summary?: {
    rangeCovered: boolean;
    selectedRecords: number;
    coverageStart: string | null;
    coverageEnd: string | null;
    clockNote: string;
    limitations: string[];
    gaps: { reason: string }[];
  };
}
interface Bootstrap {
  profile: Profile;
  authorized: boolean;
  rememberLogin?: boolean;
  loginMethod?: "pat" | "oauth";
  user?: UserIdentity | null;
  metadata: Metadata | null;
  adbPath: string | null;
  developerOptions: DeveloperOptions;
  shortcutSettings: ShortcutSettings;
  drafts: Draft[];
  submissions: Submission[];
  sessions: Session[];
  captures?: CaptureStatus[];
  draftScopes?: Record<string, DraftProjectScope>;
}
type UpdateCheckStatus =
  "idle" | "checking" | "current" | "available" | "error";
interface UpdateCheckState {
  status: UpdateCheckStatus;
  result: UpdateCheckResult | null;
  message: string;
}
type UpdateDialogState = {
  mode: "confirm" | "installing" | "error";
  result: UpdateCheckResult;
  message: string;
  progress: UpdateProgress | null;
} | null;
const stateLabels: Record<string, string> = {
  ready: "证据已固定",
  creating: "正在创建",
  create_unknown: "创建结果待核对",
  created: "缺陷已创建",
  uploading: "上传证据中",
  finalizing: "核对最终描述",
  partial: "已创建，证据未齐",
  submitted: "提交完成",
};
function displayVersion(version?: string | null) {
  return version ? `v${version.replace(/^v/i, "")}` : "—";
}
function updateStatusLabel(status: UpdateCheckStatus) {
  if (status === "checking") return "正在检查";
  if (status === "available") return "发现新版本";
  if (status === "current") return "已是最新";
  if (status === "error") return "检查失败";
  return "尚未检查";
}
function displayBytes(size?: number | null) {
  if (!size) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = size;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
function updateProgressMessage(progress: UpdateProgress) {
  if (progress.phase === "connecting")
    return "正在连接更新服务器，并读取系统代理设置。";
  if (progress.phase === "verifying")
    return "安装包下载完成，正在校验大小和 SHA-256。";
  if (progress.phase === "preparing")
    return "安装包校验通过，正在准备覆盖安装。";
  const transferred = `${displayBytes(progress.downloadedBytes)} / ${displayBytes(progress.totalBytes)}`;
  const speed = progress.bytesPerSecond
    ? ` · ${displayBytes(progress.bytesPerSecond)}/s`
    : "";
  return `正在下载安装包：${transferred}${speed}`;
}
function updateProgressDetail(progress: UpdateProgress) {
  if (progress.phase === "connecting") return "准备连接";
  const transferred = progress.totalBytes
    ? `${progress.downloadedBytes ? displayBytes(progress.downloadedBytes) : "0 B"} / ${displayBytes(progress.totalBytes)}`
    : progress.downloadedBytes
      ? displayBytes(progress.downloadedBytes)
      : "";
  const values = [
    progress.percent !== null ? `${progress.percent}%` : "",
    transferred,
    progress.phase === "downloading" && progress.bytesPerSecond
      ? `${displayBytes(progress.bytesPerSecond)}/s`
      : "",
  ];
  return values.filter(Boolean).join(" · ");
}
function sourceStatusLabel(source?: Source) {
  if (!source) return "未采集";
  if (source.status === "collected") return "自动采集";
  if (source.status === "manual") return "人工补充";
  if (source.status === "not_applicable") return "未采用";
  return "采集失败";
}
function sourceTime(source?: Source) {
  if (!source?.at) return "";
  const value = new Date(source.at);
  return Number.isNaN(value.getTime())
    ? source.at
    : value.toLocaleString("zh-CN", { hour12: false });
}
function recordRendererEvent(
  event:
    "ui_action" | "window_error" | "unhandled_rejection" | "event_loop_lag",
  action: string,
  phase: "started" | "complete" | "failed" | "observed",
  durationMs?: number,
) {
  if (!isTauri()) return;
  void invoke("record_renderer_event", {
    event,
    action,
    phase,
    durationMs: durationMs == null ? null : Math.max(0, Math.round(durationMs)),
  }).catch(() => undefined);
}
function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(initialWorkspace),
    [draft, setDraft] = useState<Draft | null>(null);
  const [tab, setTab] = useState("auth"),
    [busy, setBusy] = useState(""),
    [error, setError] = useState("");
  const { notice, setNotice } = useTransientNotice();
  const [rememberLogin, setRememberLogin] = useState(false);
  const [restoringLogin, setRestoringLogin] = useState(isTauri);
  const [token, setToken] = useState(""),
    [devices, setDevices] = useState<Device[]>([]),
    [serial, setSerial] = useState("");
  const [session, setSession] = useState<Session | null>(null),
    [preview, setPreview] = useState(false),
    [dirty, setDirty] = useState(false);
  const [remoteId, setRemoteId] = useState("");
  const [draftsToDelete, setDraftsToDelete] = useState<Draft[] | null>(null);
  const [auth, setAuth] = useState<AuthResult | null>(null);
  const [organizations, setOrganizations] = useState<Option[]>([]);
  const [projects, setProjects] = useState<Option[]>([]);
  const [projectSettings, setProjectSettings] = useState<ProjectSettings>({
    entries: [],
  });
  const [appliedProjectSettings, setAppliedProjectSettings] =
    useState<ProjectSettings>({ entries: [] });
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [settingsError, setSettingsError] = useState("");
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [settingsQuery, setSettingsQuery] = useState("");
  const [settingsEnabledOnly, setSettingsEnabledOnly] = useState(false);
  const [settingsPackageOnly, setSettingsPackageOnly] = useState(false);
  const [settingsExpanded, setSettingsExpanded] = useState(false);
  const [recordingShortcut, setRecordingShortcut] =
    useState<ShortcutAction | null>(null);
  const [shortcutErrors, setShortcutErrors] = useState<
    Partial<Record<ShortcutAction, string>>
  >({});
  const [updateCheck, setUpdateCheck] = useState<UpdateCheckState>({
    status: "idle",
    result: null,
    message: "点击检查 GitHub Release 列表。",
  });
  const [updateDialog, setUpdateDialog] = useState<UpdateDialogState>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [activeIssueTarget, setActiveIssueTarget] = useState("");
  const [backendIssues, setBackendIssues] = useState<
    { field: string; message: string }[]
  >([]);
  const [organizationId, setOrganizationId] = useState("");
  const [projectId, setProjectId] = useState("");
  const deviceReadEpoch = useRef(0);
  const selectedSerialRef = useRef("");
  const homeDeviceRefreshKey = useRef("");
  const previewTriggerRef = useRef<HTMLButtonElement>(null);
  const [submitReviewDockElement, setSubmitReviewDockElement] =
    useState<HTMLElement | null>(null);
  const [submitReviewDockHeight, setSubmitReviewDockHeight] = useState(0);
  const activeDraftRef = useRef<Draft | null>(null);
  const memoryWrite = useRef<Promise<unknown>>(Promise.resolve());
  const busyGate = useRef(false);
  const workspaceEpoch = useRef(0);
  const startupUpdateChecked = useRef(false);
  const currentShortcutPlatform = useRef(shortcutPlatform()).current;
  useEffect(() => {
    const onError = () =>
      recordRendererEvent("window_error", "window", "observed");
    const onRejection = () =>
      recordRendererEvent("unhandled_rejection", "promise", "observed");
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    let expected = performance.now() + 5_000;
    const timer = window.setInterval(() => {
      const now = performance.now();
      const delay = now - expected;
      expected = now + 5_000;
      if (delay >= 2_000 && delay <= 24 * 60 * 60 * 1_000) {
        recordRendererEvent("event_loop_lag", "renderer", "observed", delay);
      }
    }, 5_000);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
      window.clearInterval(timer);
    };
  }, []);
  const refresh = async () => {
    const b = await invoke<Bootstrap>("bootstrap");
    setBoot(b);
    return b;
  };
  useEffect(() => {
    activeDraftRef.current = draft;
  }, [draft]);
  useEffect(() => {
    const dock = submitReviewDockElement;
    if (tab !== "new" || !dock) {
      setSubmitReviewDockHeight(0);
      return;
    }
    const updateHeight = () =>
      setSubmitReviewDockHeight(Math.ceil(dock.getBoundingClientRect().height));
    updateHeight();
    const observer = new ResizeObserver(updateHeight);
    observer.observe(dock);
    window.addEventListener("resize", updateHeight);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateHeight);
    };
  }, [tab, submitReviewDockElement]);
  const run = async (
    name: string,
    work: () => Promise<unknown>,
    target?: string,
  ) => {
    if (busyGate.current) return;
    const actionStarted = performance.now();
    recordRendererEvent("ui_action", name, "started");
    busyGate.current = true;
    setBusy(name);
    setError("");
    setNotice("");
    if (target) setFieldErrors((current) => ({ ...current, [target]: "" }));
    try {
      await work();
      recordRendererEvent(
        "ui_action",
        name,
        "complete",
        performance.now() - actionStarted,
      );
    } catch (e) {
      recordRendererEvent(
        "ui_action",
        name,
        "failed",
        performance.now() - actionStarted,
      );
      if (isAuthExpired(e))
        await invalidateAuth(`授权已失效：${remoteErrorMessage(e)}`);
      else if (target)
        setFieldErrors((current) => ({
          ...current,
          [target]: typeof e === "string" ? e : String(e),
        }));
      else setError(typeof e === "string" ? e : String(e));
    } finally {
      busyGate.current = false;
      setBusy("");
    }
  };
  const currentShortcutSettings = useMemo(
    () => boot?.shortcutSettings ?? defaultShortcutSettings(),
    [boot?.shortcutSettings],
  );
  const shortcutBindings = useMemo(
    () => resolveShortcutBindings(currentShortcutSettings),
    [currentShortcutSettings],
  );
  const shortcut = (action: ShortcutAction) =>
    shortcutPresentation(action, shortcutBindings, currentShortcutPlatform);
  const persistShortcutSettings = async (settings: ShortcutSettings) => {
    const saved = await invoke<ShortcutSettings>("set_shortcut_settings", {
      settings,
    });
    setBoot((current) =>
      current ? { ...current, shortcutSettings: saved } : current,
    );
    return saved;
  };
  useEffect(() => {
    if (tab !== "settings") setRecordingShortcut(null);
  }, [tab]);
  useEffect(() => {
    const modifierKeys = new Set([
      "Alt",
      "AltGraph",
      "Control",
      "Meta",
      "Shift",
    ]);
    const onKeyDown = (event: KeyboardEvent) => {
      if (recordingShortcut) {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          setRecordingShortcut(null);
          setShortcutErrors((current) => ({
            ...current,
            [recordingShortcut]: "",
          }));
          return;
        }
        if (modifierKeys.has(event.key)) return;
        event.preventDefault();
        event.stopPropagation();
        const binding = shortcutFromKeyboardEvent(
          event,
          currentShortcutPlatform,
        );
        if (!binding) {
          setShortcutErrors((current) => ({
            ...current,
            [recordingShortcut]:
              "请按 Ctrl/Command，可额外按 Shift，再按一个字母、数字或 Enter。",
          }));
          return;
        }
        const validation = shortcutValidationError(
          binding,
          recordingShortcut,
          shortcutBindings,
        );
        if (validation) {
          setShortcutErrors((current) => ({
            ...current,
            [recordingShortcut]: validation,
          }));
          return;
        }
        const action = recordingShortcut;
        void run("保存快捷键", async () => {
          await persistShortcutSettings(
            settingsWithBinding(currentShortcutSettings, action, binding),
          );
          setRecordingShortcut(null);
          setShortcutErrors((current) => ({ ...current, [action]: "" }));
          setNotice("快捷键已保存。", "success");
        });
        return;
      }
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        tab !== "new" ||
        document.querySelector('[role="dialog"][aria-modal="true"]')
      )
        return;
      const binding = shortcutFromKeyboardEvent(event, currentShortcutPlatform);
      if (!binding) return;
      const action = shortcutDefinitions.find(
        (item) => shortcutBindings[item.id] === binding,
      )?.id;
      if (!action) return;
      const button = document.querySelector<HTMLButtonElement>(
        `[data-shortcut-action="${action}"]`,
      );
      if (!button || button.disabled || button.getClientRects().length === 0)
        return;
      event.preventDefault();
      event.stopPropagation();
      button.click();
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [
    currentShortcutPlatform,
    currentShortcutSettings,
    recordingShortcut,
    shortcutBindings,
    tab,
  ]);
  const checkForUpdates = async (showStartupPrompt = false) => {
    if (updateCheck.status === "checking") return;
    setUpdateCheck((current) => ({
      ...current,
      status: "checking",
      message: "正在读取 GitHub Release 列表…",
    }));
    try {
      const result = await invoke<UpdateCheckResult>("check_for_updates");
      setUpdateCheck({
        status: !result.ok
          ? "error"
          : result.hasUpdate
            ? "available"
            : "current",
        result,
        message: result.error || result.message,
      });
      if (showStartupPrompt && result.hasUpdate && result.canAutoUpdate) {
        setUpdateDialog({
          mode: "confirm",
          result,
          message: result.message,
          progress: null,
        });
      }
    } catch (cause) {
      setUpdateCheck({
        status: "error",
        result: null,
        message: typeof cause === "string" ? cause : String(cause),
      });
    }
  };
  const startUpdate = async (result = updateCheck.result) => {
    if (!result?.canAutoUpdate) return;
    setUpdateDialog({
      mode: "installing",
      result,
      message: "正在连接更新服务器，并读取系统代理设置。",
      progress: null,
    });
    const onProgress = new Channel<UpdateProgress>();
    onProgress.onmessage = (progress) => {
      setUpdateDialog((current) =>
        current?.mode === "installing"
          ? {
              ...current,
              progress,
              message: updateProgressMessage(progress),
            }
          : current,
      );
    };
    try {
      const installed = await invoke<UpdateInstallResult>("install_update", {
        onProgress,
      });
      if (!installed.ok) throw installed.error || installed.message;
      setUpdateDialog((current) => ({
        mode: "installing",
        result,
        message: installed.message,
        progress: current?.progress ?? null,
      }));
    } catch (cause) {
      setUpdateDialog((current) => ({
        mode: "error",
        result,
        message: typeof cause === "string" ? cause : String(cause),
        progress: current?.progress ?? null,
      }));
    }
  };
  const openReleasePage = async () => {
    try {
      await invoke("open_release_page");
      setNotice("已在系统浏览器中打开 GitHub Release 页面。");
    } catch (cause) {
      setUpdateCheck((current) => ({
        ...current,
        status: "error",
        message: typeof cause === "string" ? cause : String(cause),
      }));
    }
  };
  const setDebugEnabled = async (debugEnabled: boolean) => {
    const options = await invoke<DeveloperOptions>("set_developer_options", {
      options: { debugEnabled },
    });
    setBoot((current) =>
      current ? { ...current, developerOptions: options } : current,
    );
    setNotice(
      debugEnabled
        ? "调试已开启，后续操作会写入受限轮转日志。"
        : "调试已关闭，已停止写入详细运行日志。",
    );
  };
  useEffect(() => {
    if (!isTauri() || startupUpdateChecked.current) return;
    startupUpdateChecked.current = true;
    void checkForUpdates(true);
  }, []);
  const invalidateAuth = async (message: string) => {
    workspaceEpoch.current += 1;
    await invoke("clear_auth").catch(() => undefined);
    setDraft(null);
    setDirty(false);
    setPreview(false);
    setSession(null);
    setDevices([]);
    selectedSerialRef.current = "";
    setSerial("");
    setAuth({ status: "invalid", user: null, message });
    setRememberLogin(false);
    setOrganizations([]);
    setProjects([]);
    setProjectSettings({ entries: [] });
    setAppliedProjectSettings({ entries: [] });
    setSettingsDirty(false);
    setFieldErrors({});
    setBackendIssues([]);
    setSettingsError("");
    setSettingsQuery("");
    setSettingsEnabledOnly(false);
    setSettingsPackageOnly(false);
    setSettingsExpanded(false);
    setOrganizationId("");
    setProjectId("");
    setBoot((b) =>
      b
        ? {
            ...b,
            profile: initialWorkspace().profile,
            authorized: false,
            user: null,
            metadata: null,
            drafts: [],
            draftScopes: {},
            submissions: [],
            sessions: [],
            captures: [],
          }
        : b,
    );
    setTab("auth");
  };
  const remoteErrorMessage = (error: unknown) =>
    typeof error === "string" ? error : String(error);
  const isAuthExpired = (error: unknown) =>
    remoteAuthReaction(error) === "logout";
  const handleRemoteError = async (
    error: unknown,
    context: string,
    target?: string,
  ) => {
    const message = `${context}：${remoteErrorMessage(error)}`;
    if (isAuthExpired(error)) await invalidateAuth(message);
    else if (target)
      setFieldErrors((current) => ({ ...current, [target]: message }));
    else setError(message);
  };
  const fieldError = (path: string) =>
    fieldErrors[path] ? (
      <small className="error-text" role="alert">
        {fieldErrors[path]}
      </small>
    ) : null;
  const loadWorkspace = async (
    profile: Profile,
  ): Promise<{ valid: boolean; packageReady: boolean; profile: Profile }> => {
    const epoch = workspaceEpoch.current;
    const orgs = await invoke<Option[]>("list_organizations");
    if (epoch !== workspaceEpoch.current)
      return { valid: false, packageReady: false, profile };
    setOrganizations(orgs);
    const rememberedOrg = orgs.some((o) => o.id === profile.organizationId)
      ? profile.organizationId
      : (orgs[0]?.id ?? "");
    setOrganizationId(rememberedOrg);
    if (!rememberedOrg) {
      setProjects([]);
      setProjectSettings({ entries: [] });
      setAppliedProjectSettings({ entries: [] });
      setSettingsDirty(false);
      setSettingsQuery("");
      setSettingsEnabledOnly(false);
      setSettingsPackageOnly(false);
      setSettingsExpanded(false);
      setProjectId("");
      return { valid: false, packageReady: false, profile };
    }
    const nextProjects = await invoke<Option[]>("list_projects", {
      organizationId: rememberedOrg,
    });
    if (epoch !== workspaceEpoch.current)
      return { valid: false, packageReady: false, profile };
    setProjects(nextProjects);
    const loadedSettings = await invoke<ProjectSettings>(
      "load_project_settings",
      {
        organizationId: rememberedOrg,
      },
    );
    if (epoch !== workspaceEpoch.current)
      return { valid: false, packageReady: false, profile };
    const normalized = normalizeProjectSettings(nextProjects, loadedSettings);
    setProjectSettings(normalized);
    setAppliedProjectSettings(normalized);
    setSettingsDirty(false);
    const availableProjects = enabledProjects(nextProjects, normalized);
    const selectedProject = availableProjects.some(
      (p) => p.id === profile.projectId,
    )
      ? profile.projectId
      : (availableProjects[0]?.id ?? "");
    setProjectId(selectedProject);
    if (
      selectedProject &&
      (rememberedOrg !== profile.organizationId ||
        selectedProject !== profile.projectId)
    ) {
      const selected = await invoke<Profile>("select_project", {
        organizationId: rememberedOrg,
        projectId: selectedProject,
      });
      setBoot((current) =>
        current ? { ...current, profile: selected } : current,
      );
      return {
        valid: true,
        packageReady: Boolean(
          configuredProjectPackage(normalized, selectedProject),
        ),
        profile: selected,
      };
    }
    return {
      valid: Boolean(selectedProject),
      packageReady: Boolean(
        configuredProjectPackage(normalized, selectedProject),
      ),
      profile,
    };
  };
  const persistProjectSettings = async () => {
    if (!organizationId) return false;
    await memoryWrite.current;
    if (dirty && draft) await save();
    const epoch = workspaceEpoch.current;
    setSettingsBusy(true);
    setSettingsError("");
    try {
      const saved = await invoke<ProjectSettings>("save_project_settings", {
        organizationId,
        settings: projectSettings,
      });
      if (epoch !== workspaceEpoch.current) return false;
      const normalized = normalizeProjectSettings(projects, saved);
      setProjectSettings(normalized);
      setAppliedProjectSettings(normalized);
      setSettingsDirty(false);
      const available = enabledProjects(projects, saved);
      if (!available.length) {
        setProjectId("");
        setDraft(null);
        setBoot((current) =>
          current ? { ...current, metadata: null } : current,
        );
      } else if (!available.some((item) => item.id === projectId) && !session) {
        try {
          await selectProject(organizationId, available[0].id);
        } catch (error) {
          setProjectId("");
          setDraft(null);
          setBoot((current) =>
            current ? { ...current, metadata: null } : current,
          );
          setSettingsError(
            `项目设置已保存，但切换当前项目失败：${remoteErrorMessage(error)}`,
          );
        }
      }
      return true;
    } catch (error) {
      setSettingsError(`项目设置保存失败：${remoteErrorMessage(error)}`);
      return false;
    } finally {
      setSettingsBusy(false);
    }
  };
  const selectOrganization = async (nextOrganizationId: string) => {
    if (!nextOrganizationId || session) return;
    if (settingsDirty && !(await persistProjectSettings())) return;
    if (dirty && draft) await save();
    const epoch = ++workspaceEpoch.current;
    const nextProjects = await invoke<Option[]>("list_projects", {
      organizationId: nextOrganizationId,
    });
    const loaded = await invoke<ProjectSettings>("load_project_settings", {
      organizationId: nextOrganizationId,
    });
    if (epoch !== workspaceEpoch.current) return;
    const normalized = normalizeProjectSettings(nextProjects, loaded);
    const enabled = enabledProjects(nextProjects, normalized);
    if (!enabled.length) {
      setOrganizationId(nextOrganizationId);
      setSettingsQuery("");
      setSettingsEnabledOnly(false);
      setSettingsPackageOnly(false);
      setProjects(nextProjects);
      setProjectSettings(normalized);
      setAppliedProjectSettings(normalized);
      setProjectId("");
      setDraft(null);
      setBoot((current) =>
        current ? { ...current, metadata: null } : current,
      );
      setNotice(
        "当前组织没有启用项目，请在设置中启用至少一个项目。",
        "warning",
      );
      return;
    }
    const profile = await invoke<Profile>("select_project", {
      organizationId: nextOrganizationId,
      projectId: enabled[0].id,
    });
    setOrganizationId(nextOrganizationId);
    setSettingsQuery("");
    setSettingsEnabledOnly(false);
    setSettingsPackageOnly(false);
    setProjects(nextProjects);
    setProjectSettings(normalized);
    setAppliedProjectSettings(normalized);
    setProjectId(enabled[0].id);
    setBoot((current) =>
      current ? { ...current, profile, metadata: null } : current,
    );
    if (configuredProjectPackage(normalized, enabled[0].id)) {
      await loadProjectMetadataAndDraft(profile);
    } else {
      setDraft(null);
      setBoot((current) =>
        current ? { ...current, metadata: null } : current,
      );
    }
  };
  const retryWorkspace = async () => {
    if (!boot?.authorized) return;
    try {
      const workspace = await loadWorkspace(boot.profile);
      if (workspace.valid && profileConfigured(workspace.profile)) {
        if (workspace.packageReady)
          await loadProjectMetadataAndDraft(workspace.profile);
        else {
          setDraft(null);
          setFieldErrors((current) => {
            const next = { ...current };
            delete next.projectId;
            return next;
          });
        }
      } else {
        setDraft(null);
        setFieldErrors((current) => ({
          ...current,
          projectId: "当前项目已不可访问，请重新选择组织和项目。",
        }));
      }
    } catch (error) {
      await handleRemoteError(error, "云效项目读取失败", "projectId");
    }
  };
  const loadProjectMetadataAndDraft = async (profile: Profile) => {
    const epoch = ++workspaceEpoch.current;
    const metadata = await invoke<Metadata>("load_metadata", {
      typeId: "",
      forceRefresh: false,
    });
    if (epoch !== workspaceEpoch.current) return;
    setBoot((current) =>
      current ? { ...current, profile, metadata, authorized: true } : current,
    );
    const fresh = await invoke<Draft>("new_draft");
    if (epoch !== workspaceEpoch.current) return;
    fresh.workitemTypeId = metadata.typeId;
    let initialized = fresh;
    try {
      const memory = await invoke<Partial<SelectionMemory>>(
        "load_selection_memory",
        {
          typeId: metadata.typeId,
        },
      );
      if (epoch !== workspaceEpoch.current) return;
      initialized = applySelectionMemory(fresh, memory, metadata, profile);
      await invoke<Draft>("save_draft", {
        draft: draftSchema.parse(initialized),
      });
    } catch (error) {
      setError(`读取选择记忆失败：${remoteErrorMessage(error)}`);
    }
    if (epoch === workspaceEpoch.current) {
      setBoot((current) =>
        current
          ? {
              ...current,
              drafts: [
                initialized,
                ...current.drafts.filter((item) => item.id !== initialized.id),
              ],
              draftScopes: {
                ...(current.draftScopes ?? {}),
                [initialized.id]: {
                  organizationId: profile.organizationId,
                  projectId: profile.projectId,
                },
              },
            }
          : current,
      );
      setDraft(initialized);
      setDirty(false);
      setPreview(false);
    }
  };
  const selectProject = async (
    nextOrganizationId: string,
    nextProjectId: string,
  ) => {
    if (!nextOrganizationId || !nextProjectId) return;
    await memoryWrite.current;
    if (dirty && draft) await save();
    const profile = await invoke<Profile>("select_project", {
      organizationId: nextOrganizationId,
      projectId: nextProjectId,
    });
    setBoot((current) =>
      current
        ? { ...current, profile, authorized: true, metadata: null }
        : current,
    );
    setOrganizationId(nextOrganizationId);
    setProjectId(nextProjectId);
    if (configuredProjectPackage(appliedProjectSettings, nextProjectId)) {
      await loadProjectMetadataAndDraft(profile);
      setNotice("项目已切换，已建立新的项目草稿；旧草稿保留在本机历史中。");
    } else {
      setDraft(null);
      setNotice("项目已切换，请先在设置中补全该项目的 Android 包名。");
    }
    setDirty(false);
    setPreview(false);
  };
  useEffect(() => {
    if (!isTauri()) return;
    void (async () => {
      try {
        const b = await invoke<Bootstrap>("bootstrap");
        setBoot({ ...b, authorized: false, user: null, metadata: null });
        setDraft(null);
        if (b.rememberLogin) {
          setRememberLogin(true);
          await confirmLogin(await invoke<AuthResult>("check_auth"), true);
        } else {
          setAuth({ status: "missing", user: null, message: "请选择登录方式" });
          setTab("auth");
        }
      } catch (failure) {
        setTab("auth");
        setError(`登录恢复失败：${remoteErrorMessage(failure)}`);
      } finally {
        setRestoringLogin(false);
      }
    })();
  }, []);
  useEffect(() => {
    if (!isTauri() || !boot?.authorized || !draft || !serial || tab !== "new")
      return;
    let active = true;
    let polling = false;
    const draftId = draft.id;
    const refreshCollectedState = () => {
      if (document.hidden || busyGate.current || polling) return;
      polling = true;
      void invoke<Bootstrap>("bootstrap")
        .then((b) => {
          if (!active) return;
          setBoot(b);
          const stored = b.drafts.find((d) => d.id === draftId);
          if (!stored) return;
          setDraft((current) => {
            if (!current || current.id !== draftId) return current;
            if (
              JSON.stringify(current.attachments) ===
                JSON.stringify(stored.attachments) &&
              JSON.stringify(current.collectionErrors) ===
                JSON.stringify(stored.collectionErrors)
            )
              return current;
            setPreview(false);
            return {
              ...current,
              attachments: stored.attachments,
              collectionErrors: stored.collectionErrors,
              previewConfirmed: false,
            };
          });
        })
        .catch((e) => {
          if (active) setError(`读取本地采集进度失败：${String(e)}`);
        })
        .finally(() => {
          polling = false;
        });
    };
    const refreshWhenVisible = () => {
      if (!document.hidden) refreshCollectedState();
    };
    const timer = window.setInterval(refreshCollectedState, 3000);
    window.addEventListener("focus", refreshCollectedState);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshCollectedState);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
  }, [boot?.authorized, draft?.id, serial, tab]);
  useEffect(() => {
    setFieldErrors({});
    setBackendIssues([]);
    setActiveIssueTarget("");
    setDraftsToDelete(null);
  }, [draft?.id, organizationId]);
  const update = (path: string, value: unknown) => {
    setBackendIssues((current) =>
      current.filter((issue) => {
        if (issue.field === path) return false;
        if (!issue.field.startsWith("customFields.")) return true;
        const id = issue.field.slice("customFields.".length);
        const field = boot?.metadata?.fields.find((item) => item.id === id);
        const target = field && boot ? nativeTarget(field, boot.profile) : null;
        const nativePath =
          target === "subject"
            ? "title"
            : target === "description"
              ? "actual"
              : target;
        return nativePath !== path;
      }),
    );
    setActiveIssueTarget("");
    setDraft((current) => {
      if (!current) return current;
      const next = structuredClone(current);
      const parts = path.split(".");
      let obj: Record<string, unknown> = next as unknown as Record<
        string,
        unknown
      >;
      for (const p of parts.slice(0, -1))
        obj = obj[p] as Record<string, unknown>;
      obj[parts.at(-1)!] = value;
      if (path === "bugPage") {
        next.sources.bugPage = {
          status: "manual",
          at: new Date().toISOString(),
          detail: "人工输入",
        };
      }
      if (path !== "previewConfirmed") next.previewConfirmed = false;
      return next;
    });
    setDirty(true);
    setPreview(false);
  };
  const changeType = async (typeId: string) => {
    if (!draft || !typeId) return;
    setBackendIssues([]);
    await memoryWrite.current;
    const saved = dirty ? await save() : draft;
    const next = structuredClone(saved);
    next.workitemTypeId = typeId;
    next.assignedTo = "";
    next.sprint = "";
    next.versions = [];
    next.customFields = {};
    const metadata = await invoke<Metadata>("load_metadata", {
      typeId,
      forceRefresh: false,
    });
    setBoot((current) => (current ? { ...current, metadata } : current));
    const memory = await invoke<Partial<SelectionMemory>>(
      "load_selection_memory",
      { typeId },
    );
    const initialized = applySelectionMemory(
      next,
      memory,
      metadata,
      boot?.profile,
    );
    setDraft(initialized);
    setDirty(true);
    setPreview(false);
    rememberSelections(initialized);
  };
  const rememberSelections = (next: Draft) => {
    const selection: SelectionMemory = {
      workitemTypeId: next.workitemTypeId,
      assignedTo: next.assignedTo,
      sprint: next.sprint,
      versions: next.versions,
      kind: next.kind,
      customFields: next.customFields,
    };
    memoryWrite.current = enqueueSelectionWrite(
      memoryWrite.current,
      () => invoke("remember_selections", { selection }),
      (e) => {
        setError(`选择记忆保存失败：${typeof e === "string" ? e : String(e)}`);
      },
    );
  };
  const inspectDevice = async (selectedSerial: string) => {
    if (!selectedSerial || !draft) return;
    if (session && session.serial !== selectedSerial) {
      setFieldErrors((current) => ({
        ...current,
        "device.serial": "当前会话已绑定另一台设备，请先停止采集后再切换。",
      }));
      return;
    }
    setBackendIssues([]);
    const epoch = ++deviceReadEpoch.current;
    setFieldErrors((current) => ({ ...current, "device.serial": "" }));
    const accountEpoch = workspaceEpoch.current;
    const draftId = draft.id;
    // Persist manual edits before the backend binds the read to this draft.
    // The backend owns clearing canonical identity values on partial results;
    // do not save a synthetic cleared draft and turn observed fields manual.
    if (dirty) await save(draft);
    try {
      const snapshot = await invoke<Snapshot>("inspect_device", {
        id: draftId,
        serial: selectedSerial,
      });
      const persisted = await invoke<Bootstrap>("bootstrap");
      if (
        epoch !== deviceReadEpoch.current ||
        accountEpoch !== workspaceEpoch.current
      )
        return;
      const currentDraft = activeDraftRef.current;
      if (
        !currentDraft ||
        currentDraft.id !== draftId ||
        selectedSerialRef.current !== selectedSerial
      )
        return;
      setDraft((current) => {
        if (
          !current ||
          current.id !== draftId ||
          selectedSerialRef.current !== selectedSerial
        )
          return current;
        const merged = structuredClone(current);
        const stored = persisted.drafts.find((item) => item.id === draftId);
        const observed = stored ?? null;
        merged.app = {
          ...merged.app,
          ...(observed?.app ?? snapshot.app),
        };
        merged.device = { ...(observed?.device ?? snapshot.device) };
        merged.sources = {
          ...current.sources,
          ...(observed?.sources ?? snapshot.sources),
        };
        merged.collectionErrors = observed?.collectionErrors ?? snapshot.errors;
        merged.previewConfirmed = false;
        return merged;
      });
      setNotice(
        snapshot.errors.length
          ? "设备信息已读取，部分字段需要人工补充。"
          : "已读取当前设备和配置应用信息。",
        snapshot.errors.length ? "warning" : "success",
      );
      setDirty(false);
    } catch (error) {
      if (
        epoch === deviceReadEpoch.current &&
        accountEpoch === workspaceEpoch.current
      )
        setFieldErrors((current) => ({
          ...current,
          "device.serial": `设备信息读取失败：${remoteErrorMessage(error)}`,
        }));
    }
  };
  const refreshDevices = async (rereadSelected = true) => {
    let found: Device[];
    try {
      found = await invoke<Device[]>("list_devices");
    } catch (error) {
      selectedSerialRef.current = "";
      setSerial("");
      setDevices([]);
      throw error;
    }
    setDevices(found);
    const boundSerial = session?.serial || selectedSerialRef.current;
    const selected =
      found.find((item) => item.serial === boundSerial) ??
      found.find((item) => item.state === "device") ??
      found[0];
    if (selected) {
      selectedSerialRef.current = selected.serial;
      setSerial(selected.serial);
      if (!session && (rereadSelected || selected.serial !== boundSerial))
        await inspectDevice(selected.serial);
    } else {
      selectedSerialRef.current = "";
      setSerial("");
    }
  };
  useEffect(() => {
    if (tab !== "new") {
      homeDeviceRefreshKey.current = "";
      return;
    }
    if (!boot?.authorized || !draft) return;
    const key = `${organizationId}:${projectId}:${draft.id}`;
    if (homeDeviceRefreshKey.current === key || busyGate.current) return;
    homeDeviceRefreshKey.current = key;
    void run("刷新设备", refreshDevices, "device.serial");
  }, [tab, boot?.authorized, draft?.id, organizationId, projectId, busy]);
  const save = async (d = draft) => {
    if (!d) throw "请先创建草稿";
    const saved = await invoke<Draft>("save_draft", {
      draft: draftSchema.parse(d),
    });
    setDraft(saved);
    setBoot((current) =>
      current
        ? {
            ...current,
            drafts: current.drafts.map((item) =>
              item.id === saved.id ? saved : item,
            ),
          }
        : current,
    );
    setDirty(false);
    setNotice("草稿已保存在本机。");
    return saved;
  };
  const mutate = async (
    command: string,
    args: Record<string, unknown> = {},
  ) => {
    setBackendIssues([]);
    const d = await save();
    const next = await invoke<Draft>(command, { id: d.id, ...args });
    setDraft(next);
    setDirty(false);
    await refresh();
  };
  const importAttachments = async () => {
    try {
      await mutate("import_attachment");
      setNotice("附件已导入");
    } catch (failure) {
      if (remoteErrorMessage(failure).trim() === "已取消") {
        setNotice("已取消导入附件", "warning");
        return;
      }
      setNotice("");
      throw failure;
    }
  };
  const removeAllAttachments = async () => {
    const current = activeDraftRef.current;
    if (!current?.attachments.length) return;
    await mutate("remove_all_attachments");
    setNotice("已移除所有附件");
  };
  const newDraft = async () => {
    if (!boot?.authorized || !profileConfigured(boot.profile) || !projectId) {
      setError("请先在上方选择可访问的组织和项目。");
      return;
    }
    await memoryWrite.current;
    if (dirty && draft) await save();
    await loadProjectMetadataAndDraft(boot.profile);
    setTab("new");
  };
  const confirmLogin = async (restoredResult?: AuthResult, quiet = false) => {
    let result: AuthResult;
    try {
      result =
        restoredResult ??
        (await invoke<AuthResult>("login", {
          token,
          remember: rememberLogin,
        }));
    } catch (e) {
      setToken("");
      await invalidateAuth(
        `云效授权校验失败：${typeof e === "string" ? e : String(e)}`,
      );
      return;
    }
    setToken("");
    setAuth(result);
    if (result.status !== "valid") {
      if (restoredResult && result.status === "invalid") {
        await invoke("clear_auth");
        setRememberLogin(false);
      }
      setTab(tabForAuth(result));
      setError(
        result.status === "missing"
          ? ""
          : result.message || "登录校验失败，请重新登录。",
      );
      setBoot((b) => (b ? { ...b, authorized: false, user: result.user } : b));
      return;
    }
    setError("");
    setNotice("");
    const next = await refresh();
    setBoot({ ...next, authorized: true, user: result.user });
    setTab("new");
    let validProject = false;
    let packageReady = false;
    try {
      const workspace = await loadWorkspace(next.profile);
      validProject = workspace.valid;
      packageReady = workspace.packageReady;
      if (workspace.valid) {
        setBoot((current) =>
          current ? { ...current, profile: workspace.profile } : current,
        );
        next.profile = workspace.profile;
      }
    } catch (e) {
      await handleRemoteError(e, "云效项目读取失败", "projectId");
      return;
    }
    if (profileConfigured(next.profile) && validProject && packageReady) {
      try {
        await loadProjectMetadataAndDraft(next.profile);
      } catch (e) {
        await handleRemoteError(e, "云效元数据读取失败");
        return;
      }
    }
    setTab("new");
    if (!quiet) setNotice("登录成功，已进入新建提交。");
  };
  const logout = async () => {
    if (dirty && draft) await save();
    await memoryWrite.current;
    workspaceEpoch.current += 1;
    await invoke("clear_auth");
    setRememberLogin(false);
    memoryWrite.current = Promise.resolve();
    setSession(null);
    selectedSerialRef.current = "";
    setSerial("");
    setDevices([]);
    setDraft(null);
    setDirty(false);
    setProjectSettings({ entries: [] });
    setAppliedProjectSettings({ entries: [] });
    setSettingsDirty(false);
    setSettingsQuery("");
    setSettingsEnabledOnly(false);
    setSettingsPackageOnly(false);
    setFieldErrors({});
    setAuth({ status: "missing", user: null, message: "未登录" });
    setOrganizations([]);
    setProjects([]);
    setBoot((current) =>
      current
        ? {
            ...current,
            profile: initialWorkspace().profile,
            authorized: false,
            user: null,
            metadata: null,
            drafts: [],
            draftScopes: {},
            submissions: [],
            sessions: [],
            captures: [],
          }
        : current,
    );
    setTab("auth");
  };
  const clientIssues =
    draft && boot
      ? validateDraft(draft, boot.profile, boot.metadata, boot.authorized)
      : [];
  const issues = [
    ...new Map(
      [...clientIssues, ...backendIssues].map((issue) => [
        issue.field + "\0" + issue.message,
        issue,
      ]),
    ).values(),
  ];
  const frozen = !!boot?.submissions.some((s) => s.draftId === draft?.id);
  const capturePending = (boot?.captures ?? []).some(
    (t) => t.draftId === draft?.id && t.status === "pending",
  );
  const deviceConnected = devices.some(
    (device) => device.serial === serial && device.state === "device",
  );
  const currentSubmission = boot?.submissions.find(
    (s) => s.draftId === draft?.id,
  );
  const submittedDraftIds = new Set(
    boot?.submissions
      .filter((submission) => submission.state === "submitted")
      .map((submission) => submission.draftId),
  );
  const selectableDrafts = (boot?.drafts ?? []).filter((item) => {
    const scope = boot?.draftScopes?.[item.id];
    return (
      scope?.organizationId === organizationId &&
      scope.projectId === projectId &&
      !submittedDraftIds.has(item.id)
    );
  });
  useEffect(() => {
    if (currentSubmission?.state !== "submitted") return;
    setDraft(selectableDrafts[0] ?? null);
    setDirty(false);
    setPreview(false);
  }, [draft?.id, currentSubmission?.state]);
  const changeDraft = async (id: string) => {
    if (id === draft?.id) return;
    if (dirty && draft && !frozen) await save();
    const next = selectableDrafts.find((item) => item.id === id);
    if (!next) throw "草稿已不可用，请刷新后重试";
    setDraft(next);
    setDirty(false);
    setPreview(false);
  };
  const canDeleteDraft = (item: Draft) => {
    if (boot?.submissions.some((submission) => submission.draftId === item.id))
      return false;
    if (
      (boot?.captures ?? []).some(
        (capture) =>
          capture.draftId === item.id && capture.status === "pending",
      )
    )
      return false;
    if (item.sessionId)
      return !!boot?.sessions.find(
        (entry) => entry.id === item.sessionId && entry.stoppedAt,
      );
    return true;
  };
  const deleteDrafts = async () => {
    if (!draftsToDelete?.length) return;
    const ids = draftsToDelete.map((item) => item.id);
    const removed = new Set(ids);
    const deletingCurrent = !!draft && removed.has(draft.id);
    await memoryWrite.current;
    if (!deletingCurrent && dirty && draft && !frozen) await save();
    if (ids.length === 1) await invoke("delete_draft", { id: ids[0] });
    else await invoke("delete_drafts", { ids });
    const next = selectableDrafts.find((item) => !removed.has(item.id)) ?? null;
    setDraftsToDelete(null);
    setBoot((current) => {
      if (!current) return current;
      const scopes = { ...current.draftScopes };
      ids.forEach((id) => delete scopes[id]);
      return {
        ...current,
        drafts: current.drafts.filter((item) => !removed.has(item.id)),
        draftScopes: scopes,
      };
    });
    if (deletingCurrent) {
      setDraft(next);
      setDirty(false);
      setPreview(false);
    }
    setNotice(
      ids.length > 1 ? `已删除 ${ids.length} 个草稿。` : "草稿已删除。",
    );
    await refresh();
  };
  const draftScope = draft ? boot?.draftScopes?.[draft.id] : undefined;
  const configuredPackage = configuredProjectPackage(
    appliedProjectSettings,
    projectId,
  );
  const mainPackageReady = Boolean(projectId && configuredPackage);
  useEffect(() => {
    if (
      !draft ||
      !configuredPackage ||
      draftScope?.organizationId !== organizationId ||
      draftScope?.projectId !== projectId ||
      draft.app.packageName === configuredPackage
    )
      return;
    setDraft((current) =>
      current && current.id === draft.id
        ? {
            ...current,
            app: { ...current.app, packageName: configuredPackage },
            previewConfirmed: false,
          }
        : current,
    );
    setDirty(true);
    setPreview(false);
  }, [
    configuredPackage,
    draft?.id,
    draft?.app.packageName,
    draftScope?.organizationId,
    draftScope?.projectId,
    organizationId,
    projectId,
  ]);
  const updatePostCapture = ({ draft: stored, jobs }: PostCaptureSnapshot) => {
    setBoot((current) =>
      current
        ? {
            ...current,
            captures: [
              ...(current.captures || []).filter(
                (task) =>
                  task.kind !== "post-capture" || task.draftId !== stored.id,
              ),
              ...jobs,
            ],
          }
        : current,
    );
    setDraft((current) => {
      if (
        !current ||
        current.id !== stored.id ||
        JSON.stringify(current.attachments) ===
          JSON.stringify(stored.attachments)
      )
        return current;
      setPreview(false);
      return {
        ...current,
        attachments: stored.attachments,
        previewConfirmed: false,
      };
    });
  };
  const noEnabledProjects = Boolean(
    organizationId &&
    projects.length &&
    !enabledProjects(projects, appliedProjectSettings).length,
  );
  const normalizedSettingsQuery = settingsQuery.trim().toLocaleLowerCase();
  const visibleSettingsProjects = projectsWithEnabledFirst(
    projects,
    appliedProjectSettings,
  ).filter((project) => {
    const setting = projectSettings.entries.find(
      (item) => item.projectId === project.id,
    );
    if (settingsEnabledOnly && !setting?.enabled) return false;
    if (settingsPackageOnly && !setting?.packageName.trim()) return false;
    if (!normalizedSettingsQuery) return true;
    return `${project.name} ${setting?.packageName ?? ""}`
      .toLocaleLowerCase()
      .includes(normalizedSettingsQuery);
  });
  const displayedSettingsProjects = settingsExpanded
    ? visibleSettingsProjects
    : visibleSettingsProjects.slice(0, 3);
  const value = (path: string) =>
    path
      .split(".")
      .reduce<unknown>((v, p) => (v as Record<string, unknown>)?.[p], draft) ??
    "";
  const issueMessages = (path: string, aliases: string[] = []) =>
    issues
      .filter(
        (item) =>
          item.field === path ||
          aliases.includes(item.field) ||
          (nativeAliases[path] ?? []).includes(item.field),
      )
      .map((item) => item.message)
      .filter((message, index, all) => all.indexOf(message) === index);
  const issueRoutes = draft ? routeIssues(draft, issues) : {};
  const routedMessages = (path: string) => issueRoutes[path] ?? [];
  const nativeAliases = (() => {
    const aliases: Record<string, string[]> = {};
    for (const f of boot?.metadata?.fields ?? []) {
      const target = nativeTarget(f, boot!.profile);
      if (target) {
        const path =
          target === "subject"
            ? "title"
            : target === "description"
              ? "actual"
              : target;
        (aliases[path] ??= []).push(`customFields.${f.id}`);
      }
    }
    return aliases;
  })();
  const issueLabel = (path: string) =>
    ({
      jenkinsPackageUrl: "Jenkins包地址",
      title: "问题标题",
      actual: "问题表现",
      "app.packageName": "项目包名配置",
      "app.versionName": "versionName",
      "app.versionCode": "versionCode",
      "device.model": "设备型号",
      "device.androidVersion": "Android 版本",
      "device.totalMemKb": "总内存（kB）",
      "device.availableMemKb": "当前可用内存（kB）",
      "device.resolution": "分辨率（像素）",
      "device.density": "屏幕密度（dpi）",
      "event.occurredAt": "真实问题时间（含时区）",
      "event.sourceNote": "事件时间来源 / 修改依据",
      preconditions: "复现条件",
      steps: "复现步骤",
      expected: "期望表现",
      assignedTo: "负责人",
      sprint: "迭代",
      versions: "版本",
      project: "当前项目配置",
      scope: "迭代",
      frequency: "发生频率",
      attachments: "证据附件",
      "network.operation": "网络操作",
      "network.type": "网络类型",
      "network.error": "可观察错误",
      "network.collectionStatus": "错误码 / 请求 ID 采集状态",
      "data.type": "数据类型",
      "data.count": "数据数量",
      "data.source": "数据来源",
      "frequency.attempts": "尝试次数",
      "frequency.occurrences": "发生次数",
      workitemTypeId: "云效 Bug 类型",
      kind: "问题类型",
    })[path] ?? path;
  const issueNavigationLabel = (path: string) => {
    if (path.startsWith("attachment.")) return "证据附件";
    if (path.startsWith("customFields.")) {
      const id = path.slice("customFields.".length);
      const f = boot?.metadata?.fields.find((item) => item.id === id);
      return f?.name ?? id;
    }
    return issueLabel(path);
  };
  const issueNavigationTarget = (path: string) => {
    if (path === "scope") return "field-sprint";
    if (path === "project") return "field-projectId";
    if (path === "frequency") return "field-frequency.occurrences";
    if (path === "attachments") {
      const target = Object.keys(issueRoutes).find((key) =>
        key.startsWith("attachment"),
      );
      return target ? `field-${target}` : "field-attachment-import";
    }
    if (path.startsWith("customFields.")) {
      const target = Object.entries(nativeAliases).find(([, ids]) =>
        ids.includes(path),
      )?.[0];
      if (target) return `field-${target}`;
    }
    return `field-${path}`;
  };
  const issueNavigationGroups = groupIssuesByNavigationTarget(
    issues,
    issueNavigationTarget,
  );
  const activeIssueStillPresent = issueNavigationGroups.some(
    (group) => group.target === activeIssueTarget,
  );
  useEffect(() => {
    if (!activeIssueTarget || !activeIssueStillPresent) {
      if (activeIssueTarget) setActiveIssueTarget("");
      return;
    }
    const target = document.getElementById(activeIssueTarget);
    target?.classList.add("validation-target-error");
    return () => {
      target?.classList.remove("validation-target-error");
    };
  }, [activeIssueTarget, activeIssueStillPresent, tab]);
  const field = (
    path: string,
    label: string,
    opts: {
      type?: string;
      hint?: string;
      multi?: boolean;
      warning?: string;
      aliases?: string[];
      required?: boolean;
      fullWidth?: boolean;
      markdown?: boolean;
    } = {},
  ) => {
    const text = String(value(path));
    const messages = issueMessages(path, [
      ...(nativeAliases[path] ?? []),
      ...(opts.aliases ?? []),
    ]);
    const missingInput = (message: string) =>
      message.startsWith("请补充") ||
      message === "请逐行填写可执行的复现步骤" ||
      message === "发生次数应为 1 至尝试次数之间的整数";
    const hint =
      opts.hint ??
      (messages.some(missingInput)
        ? `请填写${label.replace(/（必填）$/, "")}`
        : undefined);
    const messagesView = messages
      .filter((message) => text.trim() || !missingInput(message))
      .map((message) => (
        <small className="error-text" key={message}>
          {message}
        </small>
      ));
    if (opts.markdown) {
      return (
        <div
          className={`field markdown-field${opts.fullWidth ? " wide" : ""}`}
          id={`field-${path}`}
          key={path}
        >
          <span>
            {label}
            {draft?.sources[path] && (
              <small className={`source ${draft.sources[path].status}`}>
                {sourceStatusLabel(draft.sources[path])}
              </small>
            )}
          </span>
          <MarkdownEditor
            value={text}
            placeholder={hint ?? "请输入 Markdown 内容"}
            disabled={frozen || !!busy}
            required={opts.required}
            onChange={(nextValue) => update(path, nextValue)}
            onPasteImage={async (file) => {
              if (!draft) throw new Error("请先创建草稿");
              const payload = await markdownImageFilePayload(file);
              const result = await invoke<MarkdownImageImportResult>(
                "import_markdown_image",
                {
                  id: draft.id,
                  field: path,
                  ...payload,
                },
              );
              setDraft((current) =>
                current?.id === result.draft.id
                  ? {
                      ...current,
                      markdownAssets: result.draft.markdownAssets,
                      previewConfirmed: false,
                    }
                  : current,
              );
              setDirty(true);
              setPreview(false);
              return result;
            }}
            onReadAsset={(assetId) => {
              if (!draft) return Promise.reject("请先创建草稿");
              return invoke<string>("read_markdown_asset", {
                id: draft.id,
                assetId,
              });
            }}
          />
          {opts.warning && (
            <small className="warning-text">{opts.warning}</small>
          )}
          {messagesView}
        </div>
      );
    }
    return (
      <label
        className={`field${opts.fullWidth ? " wide" : ""}`}
        id={`field-${path}`}
        key={path}
      >
        <span>
          {label}
          {draft?.sources[path] && (
            <small className={`source ${draft.sources[path].status}`}>
              {sourceStatusLabel(draft.sources[path])}
            </small>
          )}
        </span>
        {opts.multi ? (
          <textarea
            value={text}
            onChange={(e) => update(path, e.target.value)}
            rows={3}
            placeholder={hint}
            required={opts.required}
          />
        ) : (
          <input
            type={opts.type ?? "text"}
            value={text}
            onChange={(e) => update(path, e.target.value)}
            placeholder={hint}
            required={opts.required}
          />
        )}
        {!text.trim() && draft?.sources[path]?.status === "failed" && (
          <small className="error-text">未能自动读取该信息</small>
        )}
        {path === "bugPage" && draft?.sources.bugPage && (
          <small className="field-source-detail">
            {sourceTime(draft.sources.bugPage)
              ? `${sourceTime(draft.sources.bugPage)} · `
              : ""}
            {draft.sources.bugPage.detail}
          </small>
        )}
        {opts.warning && <small className="warning-text">{opts.warning}</small>}
        {messagesView}
      </label>
    );
  };
  const select = (
    path: string,
    label: string,
    options: Option[],
    multiple = false,
    optionError?: string,
    aliases: string[] = [],
    autoSelectFirst = true,
  ) => (
    <label className="field" id={`field-${path}`} key={path}>
      <span>{label}</span>
      <CustomSelect
        multiple={multiple}
        autoSelectFirst={autoSelectFirst}
        value={
          multiple ? ((value(path) as string[]) ?? []) : String(value(path))
        }
        options={options}
        label={label}
        onChange={(nextValue) => {
          if (path === "workitemTypeId" && !multiple) {
            void run(
              "读取类型字段",
              () => changeType(String(nextValue)),
              "workitemTypeId",
            );
            return;
          }
          update(path, nextValue);
          if (draft) {
            const next = structuredClone(draft);
            const parts = path.split(".");
            let obj = next as unknown as Record<string, unknown>;
            for (const p of parts.slice(0, -1))
              obj = obj[p] as Record<string, unknown>;
            obj[parts.at(-1)!] = nextValue;
            rememberSelections(next);
          }
        }}
      />
      {issueMessages(path, [
        ...(nativeAliases[path] ?? []),
        ...aliases,
        ...(path === "versions" || path === "sprint" ? ["scope"] : []),
      ]).map((message) => (
        <small className="error-text" key={message}>
          {message}
        </small>
      ))}
      {optionError && <small className="error-text">{optionError}</small>}
      {fieldError(path)}
    </label>
  );
  const fallbackIssues = issues.filter(
    (item) =>
      !new Set([
        "jenkinsPackageUrl",
        "title",
        "actual",
        "kind",
        "workitemTypeId",
        "assignedTo",
        "sprint",
        "versions",
        "scope",
        "project",
        "frequency",
        "event.occurredAt",
        "event.sourceNote",
        "preconditions",
        "steps",
        "expected",
        "attachments",
        "app.packageName",
        "app.versionName",
        "app.versionCode",
        "device.model",
        "device.androidVersion",
        "device.totalMemKb",
        "device.availableMemKb",
        "device.resolution",
        "device.density",
        "frequency.attempts",
        "frequency.occurrences",
        "network.operation",
        "network.type",
        "network.error",
        "network.collectionStatus",
        "data.type",
        "data.count",
        "data.source",
      ]).has(item.field) &&
      !(item.field.startsWith("customFields.") && boot?.metadata),
  );
  const dropdownFormats = new Set([
    "list",
    "multiList",
    "user",
    "multiUser",
    "sprint",
    "version",
    "tag",
  ]);
  const dynamicFields = (dropdowns: boolean) =>
    boot?.metadata ? (
      <>
        {boot.metadata.fields
          .filter((f) => !excludedField(f, boot.profile))
          .filter(
            (f) =>
              ![
                "subject",
                "description",
                "assignedTo",
                "sprint",
                "versions",
              ].includes(nativeTarget(f, boot.profile) ?? ""),
          )
          .filter(
            (f) =>
              f.showWhenCreate !== false ||
              f.required ||
              boot.profile.requiredFieldIds.includes(f.id),
          )
          .filter((f) => dropdownFormats.has(f.format) === dropdowns)
          .map((f) => {
            const path = `customFields.${f.id}`;
            const opts = ["user", "multiUser"].includes(f.format)
              ? boot.metadata!.members
              : f.format === "sprint"
                ? boot.metadata!.sprints
                : f.format === "version"
                  ? boot.metadata!.versions
                  : f.options;
            if (!supportedFormats.has(f.format))
              return (
                <div
                  className="field field-error-item"
                  id={`field-${path}`}
                  key={f.id}
                >
                  <span>{f.name}</span>
                  <small className="error-text">
                    「{f.name}」格式 {f.format}{" "}
                    未支持，请完善映射或实现后再提交。
                  </small>
                  {issueMessages(path, nativeAliases[path] ?? []).map(
                    (message) => (
                      <small className="error-text" key={message}>
                        {message}
                      </small>
                    ),
                  )}
                </div>
              );
            if (dropdownFormats.has(f.format)) {
              return select(
                path,
                f.name,
                opts,
                ["multiList", "multiUser", "version", "tag"].includes(f.format),
                f.optionError,
                [],
                !defaultUnchecked(f, boot.profile),
              );
            }
            return field(path, f.name, {
              multi: f.format === "text",
              type: f.format === "date" ? "date" : "text",
              hint:
                f.format === "int"
                  ? "请输入整数"
                  : f.format === "float"
                    ? "请输入数值"
                    : f.format === "dateTime"
                      ? "例如 2026-10-10T14:30:00+08:00"
                      : undefined,
            });
          })}
        {!dropdowns &&
          boot.metadata.fields
            .filter((f) => excludedField(f, boot.profile))
            .filter((f) => issueMessages(`customFields.${f.id}`).length > 0)
            .map((f) => (
              <div
                className="field field-error-item"
                id={`field-customFields.${f.id}`}
                key={`removed-${f.id}`}
              >
                <span>{f.name}</span>
                {issueMessages(`customFields.${f.id}`).map((message) => (
                  <small className="error-text" key={message}>
                    {message}
                  </small>
                ))}
              </div>
            ))}
        {!dropdowns &&
          issues
            .filter((item) => item.field.startsWith("customFields."))
            .filter(
              (item, index, all) =>
                all.findIndex((other) => other.field === item.field) ===
                  index &&
                !boot.metadata!.fields.some(
                  (field) => `customFields.${field.id}` === item.field,
                ),
            )
            .map((item) => (
              <div
                className="field field-error-item"
                id={`field-${item.field}`}
                key={`stale-${item.field}`}
              >
                <span>{issueNavigationLabel(item.field)}</span>
                {issueMessages(item.field).map((message) => (
                  <small className="error-text" key={message}>
                    {message}
                  </small>
                ))}
              </div>
            ))}
        {!dropdowns && !boot.metadata.fields.length && (
          <p className="wide">当前类型没有额外字段。</p>
        )}
      </>
    ) : null;
  if (!isTauri())
    return (
      <main className="browser-message">
        <h1>Android Bug Desk</h1>
        <p>请启动桌面客户端。浏览器预览没有设备、凭据和本地文件权限。</p>
        <code>npm run dev</code>
      </main>
    );
  if (restoringLogin) return <LoginRestoring />;
  if (!boot)
    return (
      <main className="browser-message">
        <h1>Android Bug Desk</h1>
        <p>{error || "正在读取本机数据…"}</p>
      </main>
    );
  return (
    <div className={`app ${boot.authorized ? "" : "login-page"}`}>
      {boot.authorized && (
        <aside>
          <div className="brand">
            <span className="brand-icon">A</span>
            <div>
              Android Bug Desk<small>测试证据工作台</small>
            </div>
          </div>
          <nav>
            {[
              ["new", "新建提交"],
              ["history", "历史提交"],
              ["settings", "设置"],
            ].map(([id, title]) => (
              <button
                className={tab === id ? "active" : ""}
                onClick={() => setTab(id)}
                key={id}
              >
                {title}
              </button>
            ))}
          </nav>
          <div className="aside-status">
            <UserProfile key={boot.user?.id || "anonymous"} user={boot.user} />
          </div>
        </aside>
      )}
      <main
        className={
          tab === "new"
            ? `home-main ${draft && mainPackageReady ? "has-submit-review-dock" : ""}`
            : tab === "history"
              ? "history-main"
              : undefined
        }
        style={
          tab === "new" && draft && mainPackageReady
            ? ({
                "--submit-review-dock-height": `${submitReviewDockHeight}px`,
              } as CSSProperties)
            : undefined
        }
      >
        {boot.authorized && tab === "new" && (
          <header className="page-header">
            <h1>新建提交</h1>
            <div className="header-selectors">
              <label className="field" id="field-projectId">
                <span>项目</span>
                <CustomSelect
                  value={projectId}
                  options={enabledProjects(projects, appliedProjectSettings)}
                  label="当前项目"
                  disabled={
                    !organizationId || noEnabledProjects || !!busy || !!session
                  }
                  onChange={(next) =>
                    void run(
                      "切换项目",
                      async () => {
                        try {
                          await selectProject(organizationId, String(next));
                        } catch (error) {
                          await handleRemoteError(
                            error,
                            "云效项目切换失败",
                            "projectId",
                          );
                        }
                      },
                      "projectId",
                    )
                  }
                />
                {issueMessages("project").map((message) => (
                  <small className="error-text" key={message}>
                    {message}
                  </small>
                ))}
                {fieldError("projectId")}
                {noEnabledProjects && (
                  <small className="error-text" role="alert">
                    当前组织没有启用项目，请前往“设置”启用至少一个项目后再新建或提交。
                  </small>
                )}
              </label>
              <label className="field header-draft-field" id="field-draft.id">
                <span>草稿</span>
                <CustomSelect
                  value={draft?.id ?? ""}
                  options={selectableDrafts.map((item) => ({
                    id: item.id,
                    name: `${item.title || "未命名草稿"} · ${new Date(item.createdAt).toLocaleString()}`,
                  }))}
                  label="当前草稿"
                  selectionIndicator="start"
                  optionAction={{
                    label: "删除",
                    disabled: (option) => {
                      const item = selectableDrafts.find(
                        (entry) => entry.id === option.id,
                      );
                      return !item || !canDeleteDraft(item);
                    },
                    onClick: (option) => {
                      const item = selectableDrafts.find(
                        (entry) => entry.id === option.id,
                      );
                      if (item && canDeleteDraft(item))
                        setDraftsToDelete([item]);
                    },
                  }}
                  footerAction={{
                    label: "删除全部",
                    disabled: !selectableDrafts.every(canDeleteDraft),
                    onClick: () => setDraftsToDelete([...selectableDrafts]),
                  }}
                  disabled={!!busy || !!session || !selectableDrafts.length}
                  onChange={(next) =>
                    void run(
                      "切换草稿",
                      () => changeDraft(String(next)),
                      "draft.id",
                    )
                  }
                />
                {fieldError("draft.id")}
              </label>
              <button
                className="header-action-button primary"
                data-shortcut-action="newIssue"
                aria-keyshortcuts={shortcut("newIssue").aria}
                disabled={
                  !profileConfigured(boot.profile) || !projectId || !!busy
                }
                title={!projectId ? "请先选择组织和项目" : undefined}
                onClick={() => void run("创建草稿", newDraft)}
              >
                ＋ 新建问题
                <ShortcutBadge shortcut={shortcut("newIssue")} />
              </button>
            </div>
          </header>
        )}
        {tab === "auth" && (
          <LoginCard
            token={token}
            setToken={setToken}
            remember={rememberLogin}
            setRemember={setRememberLogin}
            method={boot.loginMethod ?? (rememberLogin ? "pat" : "oauth")}
            error={
              error ||
              (auth && auth.status !== "missing" && auth.status !== "valid"
                ? auth.message
                : "")
            }
            busy={!!busy}
            onPatLogin={() => void run("验证登录", () => confirmLogin())}
            onAuthorized={async (result) => {
              await run("载入工作台", () => confirmLogin(result));
            }}
          />
        )}
        {tab === "settings" && (
          <>
            <section className="card settings-card">
              <div className="card-title">
                <h2>账号与组织</h2>
                <button
                  disabled={!!session || !!busy || settingsBusy}
                  onClick={() => void run("退出登录", logout)}
                >
                  退出登录
                </button>
              </div>
              <label className="field">
                <span>组织</span>
                <CustomSelect
                  value={organizationId}
                  options={organizations}
                  label="组织"
                  disabled={!!session || !!busy || settingsBusy}
                  onChange={(next) =>
                    void run(
                      "切换组织",
                      () => selectOrganization(String(next)),
                      "organizationId",
                    )
                  }
                />
                {fieldError("organizationId")}
              </label>
              <p className="muted-text">
                组织切换会保存当前草稿，并建立新的项目草稿。
              </p>
            </section>
            <section className="card settings-card">
              <div className="card-title">
                <h2>项目配置</h2>
                <div className="actions">
                  <button
                    disabled={!!session || !!busy || settingsBusy}
                    onClick={() =>
                      void run("刷新项目设置", async () => {
                        if (settingsDirty && !(await persistProjectSettings()))
                          return;
                        if (dirty && draft) await save();
                        const epoch = workspaceEpoch.current;
                        const rows = await invoke<Option[]>("list_projects", {
                          organizationId,
                        });
                        const loaded = await invoke<ProjectSettings>(
                          "load_project_settings",
                          {
                            organizationId,
                          },
                        );
                        if (epoch !== workspaceEpoch.current) return;
                        setProjects(rows);
                        const normalized = normalizeProjectSettings(
                          rows,
                          loaded,
                        );
                        setProjectSettings(normalized);
                        setAppliedProjectSettings(normalized);
                        setSettingsDirty(false);
                      })
                    }
                  >
                    刷新项目
                  </button>
                  <div className="action-item">
                    <button
                      className="primary"
                      disabled={
                        !settingsDirty || settingsBusy || !!session || !!busy
                      }
                      onClick={() =>
                        void run("保存项目设置", persistProjectSettings)
                      }
                    >
                      {settingsBusy ? "保存中…" : "保存设置"}
                    </button>
                    {settingsError &&
                      !projects.some((item) =>
                        settingsError.includes(
                          `项目 ${item.id} 的包名格式不正确`,
                        ),
                      ) && (
                        <small className="error-text" role="alert">
                          {settingsError}
                        </small>
                      )}
                  </div>
                </div>
              </div>
              <div className="project-settings-toolbar">
                <label className="settings-search">
                  <span>搜索项目</span>
                  <input
                    type="search"
                    value={settingsQuery}
                    placeholder="项目名或 Android 包名"
                    disabled={!!session || !!settingsBusy || !!busy}
                    onChange={(event) => setSettingsQuery(event.target.value)}
                  />
                </label>
                <div className="settings-filters">
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={settingsEnabledOnly}
                      disabled={!!session || !!settingsBusy || !!busy}
                      onChange={(event) =>
                        setSettingsEnabledOnly(event.target.checked)
                      }
                    />
                    已勾选
                  </label>
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={settingsPackageOnly}
                      disabled={!!session || !!settingsBusy || !!busy}
                      onChange={(event) =>
                        setSettingsPackageOnly(event.target.checked)
                      }
                    />
                    包名非空
                  </label>
                </div>
                {settingsQuery && (
                  <button
                    type="button"
                    disabled={!!settingsBusy || !!busy}
                    onClick={() => setSettingsQuery("")}
                  >
                    清空搜索
                  </button>
                )}
              </div>
              {!projects.length ? (
                <p className="muted-text">当前组织没有可访问项目。</p>
              ) : !visibleSettingsProjects.length ? (
                <div className="empty">
                  <p>没有符合搜索和筛选条件的项目。</p>
                </div>
              ) : (
                <div className="project-settings-list">
                  {displayedSettingsProjects.map((project) => {
                    const setting = projectSettings.entries.find(
                      (item) => item.projectId === project.id,
                    ) ?? {
                      projectId: project.id,
                      enabled: false,
                      packageName: "",
                    };
                    const bound = session?.serial && project.id === projectId;
                    return (
                      <div className="project-setting-row" key={project.id}>
                        <label className="check">
                          <input
                            type="checkbox"
                            checked={setting.enabled}
                            disabled={
                              !!bound || !!session || !!settingsBusy || !!busy
                            }
                            onChange={(event) => {
                              setProjectSettings((current) => ({
                                entries: current.entries.map((item) =>
                                  item.projectId === project.id
                                    ? { ...item, enabled: event.target.checked }
                                    : item,
                                ),
                              }));
                              setSettingsDirty(true);
                            }}
                          />
                          <span title={project.name}>{project.name}</span>
                        </label>
                        <div className="project-package-setting">
                          <input
                            aria-label={`${project.name} Android 包名`}
                            value={setting.packageName}
                            disabled={!!session || !!settingsBusy || !!busy}
                            placeholder="主页面功能依赖此包名，如 com.example.app"
                            onChange={(event) => {
                              setProjectSettings((current) => ({
                                entries: current.entries.map((item) =>
                                  item.projectId === project.id
                                    ? {
                                        ...item,
                                        packageName: event.target.value,
                                      }
                                    : item,
                                ),
                              }));
                              setSettingsDirty(true);
                            }}
                          />
                          {settingsError.includes(
                            `项目 ${project.id} 的包名格式不正确`,
                          ) && (
                            <small className="error-text" role="alert">
                              {settingsError}
                            </small>
                          )}
                        </div>
                      </div>
                    );
                  })}
                  {visibleSettingsProjects.length > 3 && (
                    <div className="project-settings-expand">
                      <button
                        type="button"
                        aria-expanded={settingsExpanded}
                        onClick={() =>
                          setSettingsExpanded((current) => !current)
                        }
                      >
                        {settingsExpanded
                          ? "收起（仅显示 3 项）"
                          : `展开全部（${visibleSettingsProjects.length} 项）`}
                      </button>
                    </div>
                  )}
                </div>
              )}
            </section>
            <section className="card settings-card shortcut-settings-card">
              <div className="card-title">
                <div>
                  <h2>快捷键</h2>
                  <p className="muted-text">
                    仅在“新建提交”页生效。macOS 使用 Command，Windows/Linux 使用
                    Ctrl；复制、粘贴等常用编辑组合不会被占用。
                  </p>
                </div>
                <button
                  type="button"
                  disabled={
                    !!busy ||
                    Object.keys(currentShortcutSettings.overrides).length === 0
                  }
                  onClick={() =>
                    void run("恢复默认快捷键", async () => {
                      await persistShortcutSettings(defaultShortcutSettings());
                      setRecordingShortcut(null);
                      setShortcutErrors({});
                      setNotice("快捷键已全部恢复默认。", "success");
                    })
                  }
                >
                  全部恢复默认
                </button>
              </div>
              <div className="shortcut-settings-list">
                {shortcutDefinitions.map((item) => {
                  const presentation = shortcut(item.id);
                  const recording = recordingShortcut === item.id;
                  const modified = Boolean(
                    currentShortcutSettings.overrides[item.id],
                  );
                  return (
                    <div className="shortcut-setting-row" key={item.id}>
                      <div className="shortcut-setting-copy">
                        <strong>{item.title}</strong>
                        <span>新建提交页</span>
                      </div>
                      <kbd className="shortcut-setting-value">
                        {presentation.display}
                      </kbd>
                      <button
                        type="button"
                        className={recording ? "primary" : ""}
                        aria-label={`录制“${item.title}”快捷键`}
                        aria-pressed={recording}
                        disabled={!!busy}
                        onClick={() => {
                          setRecordingShortcut(recording ? null : item.id);
                          setShortcutErrors((current) => ({
                            ...current,
                            [item.id]: "",
                          }));
                          setNotice("");
                        }}
                      >
                        {recording ? "请按快捷键…" : "录制"}
                      </button>
                      <button
                        type="button"
                        disabled={!!busy || !modified}
                        onClick={() =>
                          void run("恢复默认快捷键", async () => {
                            await persistShortcutSettings(
                              settingsWithoutBindingOverride(
                                currentShortcutSettings,
                                item.id,
                              ),
                            );
                            setRecordingShortcut(null);
                            setShortcutErrors((current) => ({
                              ...current,
                              [item.id]: "",
                            }));
                            setNotice(`“${item.title}”已恢复默认快捷键。`);
                          })
                        }
                      >
                        恢复默认
                      </button>
                      {shortcutErrors[item.id] && (
                        <small className="error-text" role="alert">
                          {shortcutErrors[item.id]}
                        </small>
                      )}
                    </div>
                  );
                })}
              </div>
              {recordingShortcut && (
                <p className="muted-text shortcut-recording-note" role="status">
                  正在录制“
                  {shortcutDefinitions.find(
                    (item) => item.id === recordingShortcut,
                  )?.title ?? recordingShortcut}
                  ”：按下新组合，或按 Escape 取消。
                </p>
              )}
            </section>
            <section
              className={`card settings-card update-card update-${updateCheck.status}`}
              data-testid="update-card"
            >
              <div className="card-title">
                <h2>检查更新</h2>
                <span className="update-status-badge">
                  {updateStatusLabel(updateCheck.status)}
                </span>
              </div>
              <div className="update-summary" aria-live="polite">
                <span className="update-indicator" aria-hidden="true" />
                <div>
                  <strong>{updateStatusLabel(updateCheck.status)}</strong>
                  <p>{updateCheck.message}</p>
                </div>
              </div>
              <div className="update-version-grid">
                <div>
                  <span>当前版本</span>
                  <strong>
                    {displayVersion(
                      updateCheck.result?.currentVersion ?? appPackage.version,
                    )}
                  </strong>
                </div>
                <div>
                  <span>最新正式版本</span>
                  <strong>
                    {displayVersion(updateCheck.result?.latestVersion)}
                  </strong>
                </div>
              </div>
              <p className="muted-text update-check-note">
                从 GitHub Release 列表检查，草稿和预发布版本不会参与比较。
                {updateCheck.result?.assetName
                  ? ` 当前系统安装包：${updateCheck.result.assetName}${
                      updateCheck.result.assetSizeBytes
                        ? `（${displayBytes(updateCheck.result.assetSizeBytes)}）`
                        : ""
                    }。`
                  : ""}
                {updateCheck.result?.checkedAtEpochMs
                  ? ` 最近检查：${new Date(
                      updateCheck.result.checkedAtEpochMs,
                    ).toLocaleString("zh-CN", { hour12: false })}`
                  : ""}
              </p>
              <div className="actions update-actions">
                <button
                  className="primary"
                  disabled={updateCheck.status === "checking"}
                  onClick={() =>
                    updateCheck.status === "available" &&
                    updateCheck.result?.canAutoUpdate
                      ? void startUpdate()
                      : void checkForUpdates()
                  }
                >
                  {updateCheck.status === "checking"
                    ? "检查中…"
                    : updateCheck.status === "available" &&
                        updateCheck.result?.canAutoUpdate
                      ? "立马更新"
                      : "检查更新"}
                </button>
                <button onClick={() => void openReleasePage()}>
                  打开 Release 页面
                </button>
              </div>
            </section>
            <section className="card settings-card developer-options-card">
              <div className="card-title">
                <div>
                  <h2>开发者选项</h2>
                  <p className="muted-text">
                    开启后记录跨平台运行阶段、耗时、ADB
                    超时和采集进度，并在新建提交页提供“软件证据”。
                  </p>
                </div>
                <label className="switch-control">
                  <input
                    type="checkbox"
                    role="switch"
                    aria-label="开启调试"
                    checked={boot.developerOptions.debugEnabled}
                    disabled={!!busy}
                    onChange={(event) =>
                      void run("更新开发者选项", () =>
                        setDebugEnabled(event.target.checked),
                      )
                    }
                  />
                  <span className="switch-track" aria-hidden="true">
                    <span className="switch-thumb" />
                  </span>
                  <strong>
                    {boot.developerOptions.debugEnabled
                      ? "调试已开启"
                      : "开启调试"}
                  </strong>
                </label>
              </div>
              <p className="muted-text diagnostics-limit-note">
                详细日志最多约 20 MB，基础日志最多约 4 MB，保留 7
                天；失败的系统报告诊断最多保留 3 份、合计约 12
                MB。软件证据包最大 25 MB，并自动替换当前草稿的旧软件证据。
              </p>
            </section>
          </>
        )}
        {tab === "new" &&
          (!mainPackageReady ? (
            <section className="card empty package-required-card" role="alert">
              <h2>请先配置当前项目包名</h2>
              <p>
                设备读取、环境快照、补采证据和提交都会使用设置页中的项目包名。
                请为当前项目填写合法的 Android 包名并保存设置。
              </p>
              <button className="primary" onClick={() => setTab("settings")}>
                前往设置
              </button>
            </section>
          ) : !draft ? (
            <section className="card empty">
              <h2>记录第一个问题</h2>
              <p>请选择云效项目，创建草稿后连接设备采集或导入已有证据。</p>
              <button
                className="primary"
                disabled={!projectId || !boot.authorized || !!busy}
                onClick={() => void run("创建草稿", newDraft)}
              >
                新建问题草稿
              </button>
            </section>
          ) : (
            <>
              {frozen && (
                <div className="banner">
                  {stateLabels[currentSubmission?.state ?? "ready"]}
                  。证据已固定，可在提交记录中核对或补传。
                </div>
              )}
              <fieldset disabled={frozen || !!busy}>
                <section className="card issue-details-card">
                  <div className="card-title">
                    <h2>问题信息与复现</h2>
                    <button
                      disabled={!boot.authorized}
                      onClick={() =>
                        void run(
                          "刷新云效配置",
                          async () => {
                            setBackendIssues([]);
                            await save();
                            const metadata = await invoke<Metadata>(
                              "load_metadata",
                              {
                                typeId: draft.workitemTypeId,
                                forceRefresh: true,
                              },
                            );
                            setBoot((current) =>
                              current ? { ...current, metadata } : current,
                            );
                          },
                          "workitemTypeId",
                        )
                      }
                    >
                      刷新云效配置
                    </button>
                  </div>
                  <div className="grid problem-fields-grid">
                    {field("title", "问题标题（必填）", {
                      aliases: ["customFields.subject"],
                      required: true,
                      fullWidth: true,
                    })}
                    {field("actual", "问题表现（必填）", {
                      markdown: true,
                      required: true,
                      fullWidth: true,
                      hint: "描述实际发生的问题表现",
                      aliases: ["customFields.description"],
                    })}
                    {field("jenkinsPackageUrl", "Jenkins包地址（选填）", {
                      fullWidth: true,
                      hint: "崩溃、ANR、多语言乱码等问题建议填写，便于开发快速定位对应构建并排查",
                    })}
                    {field("bugPage", "Bug所在页面（选填）", {
                      fullWidth: true,
                      hint: "可填写问题发生时所在页面，例如登录页、广告落地页",
                    })}
                    {field("expected", "期望表现（选填）", {
                      markdown: true,
                      hint: "需要描述清楚应该是什么表现，越清楚越好",
                    })}
                    {field("preconditions", "复现条件（选填）", {
                      markdown: true,
                      hint: "补充安装方式、账号（如有）、网络情况、涉及的云控（如有）",
                    })}
                    {field("steps", "复现步骤（选填）", {
                      markdown: true,
                      hint: "每行填写一步，描述可执行的操作",
                    })}
                  </div>
                  <div className="grid">{dynamicFields(false)}</div>
                  {issueMessages("versions").length > 0 && (
                    <label
                      className="field field-error-item"
                      id="field-versions"
                    >
                      <span>旧草稿版本</span>
                      <input readOnly value={draft.versions.join(", ")} />
                      {issueMessages("versions").map((message) => (
                        <small className="error-text" key={message}>
                          {message}
                        </small>
                      ))}
                    </label>
                  )}
                  {draft.kind === "网络" && (
                    <div className="grid">
                      {field("network.operation", "网络操作")}
                      {field("network.type", "网络类型")}
                      {field("network.error", "可观察错误")}
                      {field(
                        "network.collectionStatus",
                        "错误码 / 请求 ID 采集状态",
                      )}
                    </div>
                  )}
                  {draft.kind === "数据" && (
                    <div className="grid">
                      {field("data.type", "数据类型")}
                      {field("data.count", "数据数量")}
                      {field("data.source", "数据来源")}
                    </div>
                  )}
                  <div className="grid issue-select-grid">
                    {select(
                      "kind",
                      "问题类型",
                      [
                        "功能",
                        "UI",
                        "多语言",
                        "动态过程",
                        "崩溃",
                        "ANR",
                        "广告",
                        "性能",
                        "网络",
                        "数据",
                      ].map((v) => ({ id: v, name: v })),
                    )}
                    {select(
                      "workitemTypeId",
                      "云效 Bug 类型",
                      boot.metadata?.types ?? [],
                    )}
                    {select(
                      "assignedTo",
                      "负责人",
                      boot.metadata?.members ?? [],
                      false,
                      undefined,
                      ["customFields.assignedTo"],
                    )}
                    {select(
                      "sprint",
                      "迭代",
                      boot.metadata?.sprints ?? [],
                      false,
                      undefined,
                      ["customFields.sprint"],
                    )}
                    {dynamicFields(true)}
                  </div>
                  <div className="frequency-row">
                    <label className="check">
                      <input
                        type="checkbox"
                        checked={draft.frequency.firstDiscovery}
                        onChange={(e) =>
                          update("frequency.firstDiscovery", e.target.checked)
                        }
                      />
                      首次发现，尚未重复
                    </label>
                    {!draft.frequency.firstDiscovery && (
                      <>
                        {field("frequency.attempts", "尝试次数", {
                          hint: "尝试次数应为大于 0 的整数",
                        })}
                        {field("frequency.occurrences", "发生次数", {
                          hint: "发生次数应为 1 至尝试次数之间的整数",
                          aliases: ["frequency"],
                        })}
                      </>
                    )}
                  </div>
                </section>
              </fieldset>
              <section className="card" id="field-attachments">
                <div className="card-title">
                  <h2>
                    证据附件 <small>{draft.attachments.length} 份</small>
                  </h2>
                </div>
                <fieldset
                  className="evidence-device-connection"
                  disabled={frozen || !!busy}
                >
                  <span className="section-hint">设备连接</span>
                  <div className="device-connection-row">
                    <label className="field" id="field-device.serial">
                      <span>目标设备</span>
                      <CustomSelect
                        value={serial}
                        options={devices.map((d) => ({
                          id: d.serial,
                          name: d.label,
                        }))}
                        label="目标设备"
                        disabled={!!session || !!busy}
                        onChange={(next) => {
                          const selected = String(next);
                          selectedSerialRef.current = selected;
                          setSerial(selected);
                          void run(
                            "读取设备信息",
                            () => inspectDevice(selected),
                            "device.serial",
                          );
                        }}
                      />
                      <small>
                        {devices.find((d) => d.serial === serial)?.state ===
                        "unauthorized"
                          ? "请在手机上允许 USB 调试授权"
                          : devices.find((d) => d.serial === serial)?.state ===
                              "offline"
                            ? "设备离线，请重新连接 USB"
                            : ""}
                      </small>
                      {fieldError("device.serial")}
                    </label>
                    <div
                      className="field connection-button-field adb-control"
                      id="field-adbPath"
                    >
                      <span className="connection-button-label" aria-hidden>
                        操作
                      </span>
                      <button
                        onClick={() =>
                          void run(
                            "选择 ADB",
                            async () => {
                              await invoke("choose_adb");
                              await refresh();
                            },
                            "adbPath",
                          )
                        }
                      >
                        选择 ADB 路径
                      </button>
                      {!boot.adbPath && (
                        <small className="warning-text">
                          未检测到 Platform Tools。请选择 adb，或从 Android
                          官方网站安装。
                        </small>
                      )}
                      {fieldError("adbPath")}
                    </div>
                    <div className="field connection-button-field">
                      <span className="connection-button-label" aria-hidden>
                        操作
                      </span>
                      <button
                        disabled={!!session || !!busy}
                        onClick={() =>
                          void run("刷新设备", refreshDevices, "device.serial")
                        }
                      >
                        刷新设备
                      </button>
                    </div>
                  </div>
                </fieldset>
                <div className="actions attachment-actions">
                  <EnvironmentSnapshotEntry
                    key={`environment:${organizationId}:${projectId}:${draft.id}`}
                    draft={draft}
                    serial={serial}
                    disabled={
                      frozen ||
                      !!busy ||
                      !!session ||
                      capturePending ||
                      !deviceConnected
                    }
                    onPrepare={() => save()}
                    onUpdate={updatePostCapture}
                    onComplete={() =>
                      setNotice("设备环境快照已获取，文本已加入附件")
                    }
                    shortcut={shortcut("environmentSnapshot")}
                  />
                  <PostCaptureEntry
                    key={`post-capture:${organizationId}:${projectId}:${draft.id}`}
                    draft={draft}
                    serial={serial}
                    disabled={
                      frozen ||
                      !!busy ||
                      !!session ||
                      capturePending ||
                      !deviceConnected
                    }
                    onPrepare={() => save()}
                    onUpdate={updatePostCapture}
                    onComplete={() => setNotice("补采完成")}
                    shortcut={shortcut("postCapture")}
                  />
                  <div className="action-item" id="field-attachment-import">
                    <button
                      data-shortcut-action="importAttachments"
                      aria-keyshortcuts={shortcut("importAttachments").aria}
                      disabled={frozen || !!busy}
                      onClick={() =>
                        void run(
                          "导入附件",
                          importAttachments,
                          "attachment-import",
                        )
                      }
                    >
                      导入附件
                      <ShortcutBadge shortcut={shortcut("importAttachments")} />
                    </button>
                    {fieldError("attachment-import")}
                    {routedMessages("attachment-import").map((message) => (
                      <small className="error-text" key={message}>
                        {message}
                      </small>
                    ))}
                  </div>
                  {boot.developerOptions.debugEnabled && (
                    <div className="action-item">
                      <button
                        type="button"
                        disabled={frozen || !!busy}
                        onClick={() =>
                          void run("收集软件证据", async () => {
                            await mutate("collect_software_evidence");
                            setNotice("软件证据已生成并加入附件。");
                          })
                        }
                      >
                        软件证据
                      </button>
                    </div>
                  )}
                  {draft.attachments.length > 0 && (
                    <div className="action-item">
                      <button
                        type="button"
                        className="primary"
                        disabled={frozen || !!busy}
                        onClick={() =>
                          void run("移除所有附件", removeAllAttachments)
                        }
                      >
                        移除所有附件
                      </button>
                    </div>
                  )}
                </div>
                {(boot.captures ?? [])
                  .filter(
                    (t) =>
                      t.draftId === draft.id &&
                      t.kind !== "post-capture" &&
                      t.kind !== "software-diagnostics",
                  )
                  .map((t) => (
                    <div className="capture-status" role="status" key={t.id}>
                      <strong>
                        {t.kind === "log-window" ? "问题日志窗口" : "系统报告"}{" "}
                        ·{" "}
                        {{
                          pending: "采集中",
                          complete: "采集结束",
                          interrupted: "采集中断",
                          failed: "采集失败",
                        }[t.status] ?? t.status}
                      </strong>
                      {t.status === "pending" && t.window && (
                        <p>
                          正在保留问题后 30
                          秒日志，请保持设备连接。完成后预览并选择要上传的副本。
                        </p>
                      )}
                      {t.error && <p>{t.error}</p>}
                      {t.summary && (
                        <>
                          <p>
                            提取 {t.summary.selectedRecords} 条记录 ·{" "}
                            {t.summary.rangeCovered
                              ? "观察到请求窗口的时间边界"
                              : "覆盖存在缺口或不确定项"}
                          </p>
                          <p>
                            实际日志边界：{t.summary.coverageStart ?? "未知"} ～{" "}
                            {t.summary.coverageEnd ?? "未知"}
                          </p>
                          <details>
                            <summary>查看时间依据与缺口</summary>
                            <p>{t.summary.clockNote}</p>
                            {[
                              ...t.summary.limitations,
                              ...t.summary.gaps.map((g) => g.reason),
                            ].map((v, n) => (
                              <p key={n}>{v}</p>
                            ))}
                          </details>
                        </>
                      )}
                    </div>
                  ))}
                {draft.attachments.map((a) => (
                  <AttachmentCard
                    key={a.id}
                    attachment={a}
                    issueRoutes={issueRoutes}
                    disabled={frozen || !!busy}
                    onSaveCoverage={(start, end) =>
                      void run("保存覆盖时间", () =>
                        mutate("save_attachment_coverage", {
                          attachmentId: a.id,
                          coverageStart: start,
                          coverageEnd: end,
                        }),
                      )
                    }
                    onRedact={(literal) =>
                      void run("生成脱敏副本", () =>
                        mutate("redact_log", { attachmentId: a.id, literal }),
                      )
                    }
                    onRemove={() =>
                      void run("移除附件", () =>
                        mutate("remove_attachment", { attachmentId: a.id }),
                      )
                    }
                    previewError={fieldErrors[`attachment.${a.id}.preview`]}
                    previewBusy={!!busy}
                    onPreview={() =>
                      void run(
                        "打开附件",
                        async () =>
                          await invoke("preview_attachment", {
                            id: draft.id,
                            attachmentId: a.id,
                          }),
                        `attachment.${a.id}.preview`,
                      )
                    }
                  />
                ))}
              </section>
              <section
                className="card submit-review-dock"
                ref={setSubmitReviewDockElement}
                aria-label="提交前核对"
              >
                <div className="submit-review-heading">
                  <h2>提交前核对</h2>
                  {issueNavigationGroups.length > 0 ? (
                    <div className="missing-list">
                      {issueNavigationGroups.map((group) => (
                        <button
                          key={group.target}
                          className={
                            activeIssueTarget === group.target ? "active" : ""
                          }
                          aria-current={
                            activeIssueTarget === group.target
                              ? "true"
                              : undefined
                          }
                          onClick={() => {
                            setActiveIssueTarget(group.target);
                            if (group.field === "app.packageName") {
                              setPreview(false);
                              setTab("settings");
                              return;
                            }
                            document
                              .getElementById(group.target)
                              ?.scrollIntoView({
                                behavior: "smooth",
                                block: "center",
                              });
                          }}
                        >
                          {issueNavigationLabel(group.field)}（1项）
                        </button>
                      ))}
                    </div>
                  ) : (
                    <p className="success-text">
                      基础信息已齐全，可以进入预览。
                    </p>
                  )}
                </div>
                {[...new Set(fallbackIssues.map((item) => item.field))].map(
                  (path) => (
                    <div
                      className="field field-error-item"
                      id={`field-${path}`}
                      key={path}
                    >
                      <span>待核对的校验项</span>
                      <input
                        readOnly
                        value={path}
                        aria-label="待核对的校验项"
                      />
                      {issueMessages(path).map((message) => (
                        <small className="error-text" key={message}>
                          {message}
                        </small>
                      ))}
                    </div>
                  ),
                )}
                {draft.collectionErrors.length > 0 && (
                  <details>
                    <summary>
                      采集失败与证据缺口（{draft.collectionErrors.length}）
                    </summary>
                    <ul>
                      {draft.collectionErrors.map((s, i) => (
                        <li key={i}>{s}</li>
                      ))}
                    </ul>
                  </details>
                )}
                <div className="actions">
                  <button
                    data-shortcut-action="saveDraft"
                    aria-keyshortcuts={shortcut("saveDraft").aria}
                    disabled={frozen || !!busy}
                    onClick={() =>
                      void run("保存草稿", async () => {
                        await save();
                        await refresh();
                      })
                    }
                  >
                    保存草稿
                    <ShortcutBadge shortcut={shortcut("saveDraft")} />
                  </button>
                  <button
                    data-shortcut-action="exportEvidence"
                    aria-keyshortcuts={shortcut("exportEvidence").aria}
                    disabled={!!busy}
                    onClick={() =>
                      void run("导出证据包", async () => {
                        if (!frozen) await save();
                        const path = await invoke<string>("export_evidence", {
                          id: draft.id,
                          includeMedia: true,
                        });
                        setNotice(`完整离线证据包已导出：${path}`);
                      })
                    }
                  >
                    导出本地证据
                    <ShortcutBadge shortcut={shortcut("exportEvidence")} />
                  </button>
                  <button
                    ref={previewTriggerRef}
                    className="primary"
                    data-shortcut-action="submissionPreview"
                    aria-keyshortcuts={shortcut("submissionPreview").aria}
                    disabled={
                      frozen ||
                      !!busy ||
                      capturePending ||
                      clientIssues.length > 0
                    }
                    onClick={() =>
                      void run("核对主进程校验", async () => {
                        const d = await save();
                        const errors = await invoke<
                          { field: string; message: string }[]
                        >("validate_draft", { id: d.id });
                        setBackendIssues(errors);
                        if (errors.length > 0) {
                          setPreview(false);
                          return;
                        }
                        setPreview(true);
                      })
                    }
                  >
                    进入提交预览
                    <ShortcutBadge shortcut={shortcut("submissionPreview")} />
                  </button>
                </div>
              </section>
              {preview && (
                <AppDialog
                  title="提交预览"
                  className="preview-dialog"
                  onClose={() => setPreview(false)}
                  returnFocusRef={previewTriggerRef}
                >
                  <section className="preview">
                    <dl>
                      <dt>云效项目</dt>
                      <dd>{boot.profile.name}</dd>
                      <dt>负责人</dt>
                      <dd>
                        {boot.metadata?.members.find(
                          (m) => m.id === draft.assignedTo,
                        )?.name ?? "未选择"}
                      </dd>
                      <dt>范围</dt>
                      <dd>
                        版本 {draft.versions.join(", ")} · 迭代 {draft.sprint}
                      </dd>
                      <dt>Jenkins包地址</dt>
                      <dd className="preserve">{draft.jenkinsPackageUrl}</dd>
                      <dt>问题标题</dt>
                      <dd>{draft.title}</dd>
                      <dt>Bug所在页面</dt>
                      <dd className="preserve">
                        {draft.bugPage || "未填写"}
                        {draft.sources.bugPage && (
                          <small className="preview-source-detail">
                            {sourceStatusLabel(draft.sources.bugPage)}
                            {sourceTime(draft.sources.bugPage)
                              ? ` · ${sourceTime(draft.sources.bugPage)}`
                              : ""}
                            {draft.sources.bugPage.detail
                              ? ` · ${draft.sources.bugPage.detail}`
                              : ""}
                          </small>
                        )}
                        {!draft.sources.bugPage && (
                          <small className="preview-source-detail">
                            来源未记录
                          </small>
                        )}
                      </dd>
                      <dt>复现条件</dt>
                      <dd>
                        <MarkdownPreview
                          value={draft.preconditions}
                          onReadAsset={(assetId) =>
                            invoke("read_markdown_asset", {
                              id: draft.id,
                              assetId,
                            })
                          }
                        />
                      </dd>
                      <dt>复现步骤</dt>
                      <dd>
                        <MarkdownPreview
                          value={draft.steps}
                          onReadAsset={(assetId) =>
                            invoke("read_markdown_asset", {
                              id: draft.id,
                              assetId,
                            })
                          }
                        />
                      </dd>
                      <dt>期望表现 / 问题表现</dt>
                      <dd>
                        <MarkdownPreview
                          value={draft.expected}
                          onReadAsset={(assetId) =>
                            invoke("read_markdown_asset", {
                              id: draft.id,
                              assetId,
                            })
                          }
                        />
                        <hr />
                        <MarkdownPreview
                          value={draft.actual}
                          onReadAsset={(assetId) =>
                            invoke("read_markdown_asset", {
                              id: draft.id,
                              assetId,
                            })
                          }
                        />
                      </dd>
                      <dt>设备 / 应用 / 时间</dt>
                      <dd className="preserve">
                        {JSON.stringify(
                          {
                            device: draft.device,
                            app: draft.app,
                            event: draft.event,
                          },
                          null,
                          2,
                        )}
                      </dd>
                      <dt>将上传</dt>
                      <dd>
                        {draft.attachments.map((a) => a.name).join("、")}
                        ，以及不可变证据 ZIP
                      </dd>
                    </dl>
                    <div className="preview-submit-actions">
                      <button
                        className="primary"
                        disabled={!!busy || capturePending}
                        onClick={() =>
                          void run("提交 Bug 并核对附件", async () => {
                            const d = await save();
                            const s = await invoke<Submission>("submit_bug", {
                              id: d.id,
                            });
                            await refresh();
                            setPreview(false);
                            setTab("history");
                            setNotice(stateLabels[s.state] ?? s.state);
                          })
                        }
                      >
                        提交 Bug
                      </button>
                    </div>
                  </section>
                </AppDialog>
              )}
            </>
          ))}
        {tab === "history" && (
          <HistoryView
            key={`${boot.user?.id}:${organizationId}:${projectId}`}
            profile={boot.profile}
            metadata={boot.metadata}
            drafts={boot.drafts}
            submissions={boot.submissions}
            configured={!!projectId && profileConfigured(boot.profile)}
            busy={!!busy}
            refresh={() => run("刷新记录", refresh)}
            remoteError={(e) => handleRemoteError(e, "云效历史读取失败")}
            renderSubmission={(s) => (
              <article className="submission" key={s.draftId}>
                <div className="card-title">
                  <h3>{boot.drafts.find((d) => d.id === s.draftId)?.title}</h3>
                  <span className="pill">
                    {stateLabels[s.state] ?? s.state}
                  </span>
                </div>
                <p>
                  工作项 {s.identifier || s.workitemId || "尚未获得确定 ID"} ·{" "}
                  {s.uploads.filter((a) => a.state === "confirmed").length}/
                  {s.uploads.length} 份附件已核对
                </p>
                <code>evidenceId: {s.evidence.id}</code>
                {s.error && <p className="error-text">{s.error}</p>}
                <div className="actions">
                  {s.url && (
                    <>
                      <button
                        onClick={() =>
                          void run("打开云效", () =>
                            invoke("open_receipt", { id: s.draftId }),
                          )
                        }
                      >
                        打开云效
                      </button>
                      <button
                        onClick={() =>
                          void run("复制链接", () =>
                            invoke("copy_receipt", {
                              id: s.draftId,
                              repair: false,
                            }),
                          )
                        }
                      >
                        复制链接
                      </button>
                    </>
                  )}
                  <button
                    onClick={() =>
                      void run("复制修复输入", () =>
                        invoke("copy_receipt", {
                          id: s.draftId,
                          repair: true,
                        }),
                      )
                    }
                  >
                    复制修复任务输入
                  </button>
                  {s.state === "partial" && (
                    <button
                      className="primary"
                      onClick={() =>
                        void run("核对并补传证据", async () => {
                          await invoke("submit_bug", { id: s.draftId });
                          await refresh();
                        })
                      }
                    >
                      核对并恢复
                    </button>
                  )}
                  {s.state === "create_unknown" && (
                    <>
                      <input
                        placeholder="可选：人工确认的工作项 ID"
                        value={remoteId}
                        onChange={(e) => setRemoteId(e.target.value)}
                      />
                      <button
                        onClick={() =>
                          void run("核对创建结果", async () => {
                            await invoke("reconcile_submission", {
                              id: s.draftId,
                              remoteId: remoteId || null,
                            });
                            await refresh();
                          })
                        }
                      >
                        核对已有缺陷
                      </button>
                    </>
                  )}
                </div>
              </article>
            )}
          />
        )}
        {draftsToDelete && (
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-draft-title"
          >
            <section className="delete-draft-dialog">
              <h2 id="delete-draft-title">
                {draftsToDelete.length > 1
                  ? "删除当前项目全部草稿？"
                  : "删除草稿？"}
              </h2>
              <p>
                {draftsToDelete.length > 1
                  ? `将删除当前项目的 ${draftsToDelete.length} 个草稿`
                  : `将删除「${draftsToDelete[0]?.title || "未命名草稿"}」`}
                及其本地草稿记录，无法撤销。已导出的证据包不受影响。
              </p>
              <div className="actions">
                <button
                  disabled={!!busy}
                  onClick={() => setDraftsToDelete(null)}
                >
                  取消
                </button>
                <button
                  className="danger"
                  disabled={!!busy}
                  onClick={() => {
                    void run(
                      "删除草稿",
                      async () => {
                        try {
                          await deleteDrafts();
                        } catch (error) {
                          setDraftsToDelete(null);
                          throw error;
                        }
                      },
                      "draft.id",
                    );
                  }}
                >
                  确认删除
                </button>
              </div>
            </section>
          </div>
        )}
      </main>
      {updateDialog && (
        <AppDialog
          title={
            updateDialog.mode === "error"
              ? "更新失败"
              : updateDialog.mode === "installing"
                ? "正在更新"
                : "发现新版本"
          }
          className="update-dialog"
          onClose={() => setUpdateDialog(null)}
        >
          <div className="update-dialog-body" aria-live="polite">
            {updateDialog.mode === "installing" && (
              <span className="update-spinner" aria-hidden="true" />
            )}
            <div>
              <p>{updateDialog.message}</p>
              <dl className="update-dialog-details">
                <div>
                  <dt>版本</dt>
                  <dd>
                    {displayVersion(updateDialog.result.currentVersion)} →{" "}
                    {displayVersion(updateDialog.result.latestVersion)}
                  </dd>
                </div>
                {updateDialog.result.assetName && (
                  <div>
                    <dt>安装包</dt>
                    <dd>
                      {updateDialog.result.assetName}
                      {updateDialog.result.assetSizeBytes
                        ? ` · ${displayBytes(updateDialog.result.assetSizeBytes)}`
                        : ""}
                    </dd>
                  </div>
                )}
              </dl>
              {updateDialog.mode === "installing" && updateDialog.progress && (
                <div className="update-download-status">
                  <div
                    className="update-progress-track"
                    role="progressbar"
                    aria-label="更新下载进度"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={updateDialog.progress.percent ?? undefined}
                  >
                    <span
                      style={{
                        width: `${updateDialog.progress.percent ?? 0}%`,
                      }}
                    />
                  </div>
                  <small>{updateProgressDetail(updateDialog.progress)}</small>
                </div>
              )}
            </div>
          </div>
          {updateDialog.mode === "confirm" && (
            <div className="actions update-dialog-actions">
              <button onClick={() => setUpdateDialog(null)}>取消</button>
              <button
                className="primary"
                onClick={() => void startUpdate(updateDialog.result)}
              >
                立马更新
              </button>
            </div>
          )}
          {updateDialog.mode === "installing" && (
            <p className="muted-text update-install-note">
              下载完成后应用会自动退出、覆盖安装并重新打开。
            </p>
          )}
          {updateDialog.mode === "error" && (
            <div className="actions update-dialog-actions">
              <button onClick={() => void openReleasePage()}>
                打开 Release 页面
              </button>
              <button className="primary" onClick={() => setUpdateDialog(null)}>
                关闭
              </button>
            </div>
          )}
        </AppDialog>
      )}
      <NotificationToast
        notification={
          boot.authorized && error
            ? { id: `error:${error}`, message: error, kind: "error" }
            : busy
              ? { id: `busy:${busy}`, message: `${busy}…`, kind: "loading" }
              : boot.authorized
                ? notice
                : null
        }
        onDismiss={() => {
          setError("");
          setNotice("");
        }}
      />
    </div>
  );
}
function AttachmentCard({
  attachment: a,
  issueRoutes,
  disabled,
  onSaveCoverage,
  onRemove,
  onPreview,
  previewError,
  previewBusy,
  onRedact,
}: {
  attachment: Attachment;
  issueRoutes: Record<string, string[]>;
  disabled: boolean;
  onSaveCoverage: (s: string, e: string) => void;
  onRemove: () => void;
  onPreview: () => void;
  previewError?: string;
  previewBusy: boolean;
  onRedact: (literal: string) => void;
}) {
  const [start, setStart] = useState(a.coverageStart),
    [end, setEnd] = useState(a.coverageEnd);
  const [literal, setLiteral] = useState("");
  const attachmentDescription = splitPostCaptureAttachmentDescription(
    a.name,
    a.source,
  );
  const messages = (suffix: "coverageStart" | "coverageEnd" | "confirmed") =>
    (issueRoutes[attachmentIssueKey(a, suffix)] ?? [])
      .filter(
        (message) =>
          suffix === "confirmed" ||
          (suffix === "coverageStart"
            ? a.coverageStart
            : a.coverageEnd
          ).trim() ||
          !message.startsWith("请填写日志的真实覆盖"),
      )
      .map((message) => (
        <small className="error-text" key={message}>
          {message}
        </small>
      ));
  return (
    <article className="attachment">
      <div className="attachment-body">
        <div>
          <strong>{a.name}</strong>
          <small className="attachment-meta">
            {
              {
                log: "日志",
                image: "图片",
                video: "视频",
                report: "报告",
                sample: "数据文件",
              }[a.role]
            }{" "}
            · {(a.size / 1024 / 1024).toFixed(2)} MB
            {!attachmentDescription.captureItem &&
              attachmentDescription.description && (
                <> · {attachmentDescription.description}</>
              )}
          </small>
          {attachmentDescription.captureItem && (
            <>
              <small className="attachment-capture-item">
                <span>采集项</span>
                <span>{attachmentDescription.captureItem}</span>
              </small>
              {attachmentDescription.description && (
                <small className="attachment-source">
                  {attachmentDescription.description}
                </small>
              )}
            </>
          )}
          <code>SHA256 {a.sha256}</code>
          <div id={`field-${attachmentIssueKey(a, "confirmed")}`}>
            {messages("confirmed")}
          </div>
          {a.role === "log" && (
            <>
              <details className="redaction-control">
                <summary>生成脱敏副本</summary>
                <p>
                  指定需要替换的文字，原文件保留，草稿中改用脱敏副本；只替换完全匹配的文字。
                </p>
                <label className="field">
                  <span>需要替换的文字</span>
                  <input
                    type="password"
                    autoComplete="off"
                    disabled={disabled}
                    value={literal}
                    placeholder="输入要精确匹配并替换的文字"
                    onChange={(e) => setLiteral(e.target.value)}
                  />
                </label>
                <button
                  disabled={disabled || !literal}
                  onClick={() => {
                    onRedact(literal);
                    setLiteral("");
                  }}
                >
                  生成副本
                </button>
              </details>
              <div className="grid">
                <label
                  className="field"
                  id={`field-${attachmentIssueKey(a, "coverageStart")}`}
                >
                  <span>真实覆盖起点（含时区）</span>
                  <input
                    disabled={disabled}
                    value={start}
                    placeholder="例如 2026-10-10T14:30:00+08:00"
                    onChange={(e) => setStart(e.target.value)}
                  />
                  {messages("coverageStart")}
                </label>
                <label
                  className="field"
                  id={`field-${attachmentIssueKey(a, "coverageEnd")}`}
                >
                  <span>真实覆盖终点（含时区）</span>
                  <input
                    disabled={disabled}
                    value={end}
                    placeholder="填写不早于起点的时间，包含时区"
                    onChange={(e) => setEnd(e.target.value)}
                  />
                  {messages("coverageEnd")}
                </label>
              </div>
              <button
                disabled={disabled}
                onClick={() => onSaveCoverage(start, end)}
              >
                保存覆盖时间
              </button>
            </>
          )}
        </div>
        <div className="attachment-tools">
          <div className="action-item" id={`field-attachment.${a.id}.preview`}>
            <button disabled={previewBusy} onClick={onPreview}>
              预览
            </button>
            {previewError && (
              <small className="error-text">{previewError}</small>
            )}
          </div>
          <button disabled={disabled} onClick={onRemove}>
            移除
          </button>
        </div>
      </div>
    </article>
  );
}
createRoot(document.getElementById("root")!).render(<App />);

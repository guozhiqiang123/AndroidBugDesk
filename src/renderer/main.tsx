import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke, convertFileSrc, isTauri } from "@tauri-apps/api/core";
import {
  draftSchema,
  type Draft,
  type Profile,
  type Metadata,
  type Submission,
  type Device,
  type Session,
  type Attachment,
  type Option,
} from "../shared/domain";
import type {
  AuthResult,
  HistoryPage,
  SelectionMemory,
  UserIdentity,
} from "../shared/workspace";
import {
  nativeTarget,
  validateDraft,
  supportedFormats,
} from "../shared/validation";
import { enqueueSelectionWrite, tabForAuth } from "./session";
import { applySelectionMemory, profileConfigured } from "./session";
import { initialWorkspace, remoteAuthReaction } from "./session";
import "./style.css";
import { UserProfile } from "./user-profile";

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
  user?: UserIdentity | null;
  metadata: Metadata | null;
  adbPath: string | null;
  drafts: Draft[];
  submissions: Submission[];
  sessions: Session[];
  captures?: CaptureStatus[];
}
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
function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(initialWorkspace),
    [draft, setDraft] = useState<Draft | null>(null);
  const [tab, setTab] = useState("auth"),
    [busy, setBusy] = useState(""),
    [notice, setNotice] = useState(""),
    [error, setError] = useState("");
  const [token, setToken] = useState(""),
    [devices, setDevices] = useState<Device[]>([]),
    [serial, setSerial] = useState("");
  const [packageName, setPackageName] = useState(""),
    [buildUrl, setBuildUrl] = useState(""),
    [business, setBusiness] = useState("");
  const [session, setSession] = useState<Session | null>(null),
    [preview, setPreview] = useState(false),
    [log, setLog] = useState(""),
    [dirty, setDirty] = useState(false);
  const [remoteId, setRemoteId] = useState("");
  const [auth, setAuth] = useState<AuthResult | null>(null);
  const [remoteHistory, setRemoteHistory] = useState<HistoryPage | null>(null);
  const [remoteHistoryPage, setRemoteHistoryPage] = useState(1);
  const [remoteHistoryError, setRemoteHistoryError] = useState("");
  const [organizations, setOrganizations] = useState<Option[]>([]);
  const [projects, setProjects] = useState<Option[]>([]);
  const [organizationId, setOrganizationId] = useState("");
  const [projectId, setProjectId] = useState("");
  const memoryWrite = useRef<Promise<unknown>>(Promise.resolve());
  const busyGate = useRef(false);
  const workspaceEpoch = useRef(0);
  const refresh = async () => {
    const b = await invoke<Bootstrap>("bootstrap");
    setBoot(b);
    return b;
  };
  const run = async (name: string, work: () => Promise<unknown>) => {
    if (busyGate.current) return;
    busyGate.current = true;
    setBusy(name);
    setError("");
    setNotice("");
    try {
      await work();
    } catch (e) {
      if (isAuthExpired(e))
        await invalidateAuth(`授权已失效：${remoteErrorMessage(e)}`);
      else setError(typeof e === "string" ? e : String(e));
    } finally {
      busyGate.current = false;
      setBusy("");
    }
  };
  const invalidateAuth = async (message: string) => {
    workspaceEpoch.current += 1;
    await invoke("clear_auth").catch(() => undefined);
    setDraft(null);
    setDirty(false);
    setPreview(false);
    setSession(null);
    setDevices([]);
    setSerial("");
    setPackageName("");
    setBuildUrl("");
    setBusiness("");
    setAuth({ status: "invalid", user: null, message });
    setOrganizations([]);
    setProjects([]);
    setRemoteHistory(null);
    setRemoteHistoryError(message);
    setBoot((b) =>
      b ? { ...b, authorized: false, user: null, metadata: null } : b,
    );
    setTab("auth");
  };
  const remoteErrorMessage = (error: unknown) =>
    typeof error === "string" ? error : String(error);
  const isAuthExpired = (error: unknown) =>
    remoteAuthReaction(error) === "logout";
  const handleRemoteError = async (error: unknown, context: string) => {
    const message = `${context}：${remoteErrorMessage(error)}`;
    if (isAuthExpired(error)) await invalidateAuth(message);
    else setError(message);
  };
  const loadWorkspace = async (profile: Profile): Promise<boolean> => {
    const orgs = await invoke<Option[]>("list_organizations");
    setOrganizations(orgs);
    const rememberedOrg = orgs.some((o) => o.id === profile.organizationId)
      ? profile.organizationId
      : "";
    setOrganizationId(rememberedOrg);
    if (!rememberedOrg) {
      setProjects([]);
      setProjectId("");
      return false;
    }
    const nextProjects = await invoke<Option[]>("list_projects", {
      organizationId: rememberedOrg,
    });
    setProjects(nextProjects);
    const validProject = nextProjects.some((p) => p.id === profile.projectId);
    setProjectId(validProject ? profile.projectId : "");
    return validProject;
  };
  const retryWorkspace = async () => {
    if (!boot?.authorized) return;
    try {
      const validProject = await loadWorkspace(boot.profile);
      if (validProject && profileConfigured(boot.profile))
        await loadProjectMetadataAndDraft(boot.profile);
      else {
        setDraft(null);
        setError("当前项目已不可访问，请重新选择组织和项目。");
      }
    } catch (error) {
      await handleRemoteError(error, "云效项目读取失败");
    }
  };
  const loadProjectMetadataAndDraft = async (profile: Profile) => {
    const epoch = ++workspaceEpoch.current;
    const metadata = await invoke<Metadata>("load_metadata", { typeId: "" });
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
    const b = await refresh();
    setBoot({ ...b, profile, authorized: true, metadata: null });
    setOrganizationId(nextOrganizationId);
    setProjectId(nextProjectId);
    await loadProjectMetadataAndDraft(profile);
    setDirty(false);
    setPreview(false);
    setNotice("项目已切换，已建立新的项目草稿；旧草稿保留在本机历史中。");
  };
  useEffect(() => {
    if (isTauri())
      void run("打开登录页", async () => {
        const b = await invoke<Bootstrap>("bootstrap");
        setAuth({
          status: "missing",
          user: null,
          message: "请输入个人访问令牌",
        });
        setBoot({ ...b, authorized: false, user: null, metadata: null });
        setDraft(null);
        setTab("auth");
      });
  }, []);
  useEffect(() => {
    if (!isTauri() || tab !== "history" || !boot?.authorized) return;
    void run("读取云效历史", async () => {
      setRemoteHistoryError("");
      try {
        const page = await invoke<HistoryPage>("list_remote_history", {
          page: 1,
        });
        setRemoteHistory(page);
        setRemoteHistoryPage(1);
      } catch (e) {
        setRemoteHistory(null);
        const message = `云效历史读取失败：${remoteErrorMessage(e)}`;
        setRemoteHistoryError(message);
        await handleRemoteError(e, "云效历史读取失败");
      }
    });
  }, [tab, boot?.authorized]);
  useEffect(() => {
    if (!isTauri() || !boot?.authorized || !draft?.sessionId) return;
    let active = true;
    const draftId = draft.id;
    const timer = window.setInterval(() => {
      if (busyGate.current) return;
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
        });
    }, 1500);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [boot?.authorized, draft?.id, draft?.sessionId]);
  const update = (path: string, value: unknown) => {
    if (!draft) return;
    const next = structuredClone(draft);
    const parts = path.split(".");
    let obj: Record<string, unknown> = next as unknown as Record<
      string,
      unknown
    >;
    for (const p of parts.slice(0, -1)) obj = obj[p] as Record<string, unknown>;
    obj[parts.at(-1)!] = value;
    if (path !== "previewConfirmed") next.previewConfirmed = false;
    setDraft(next);
    setDirty(true);
    setPreview(false);
  };
  const changeType = async (typeId: string) => {
    if (!draft || !typeId) return;
    await memoryWrite.current;
    const saved = dirty ? await save() : draft;
    const next = structuredClone(saved);
    next.workitemTypeId = typeId;
    next.assignedTo = "";
    next.sprint = "";
    next.versions = [];
    next.customFields = {};
    const metadata = await invoke<Metadata>("load_metadata", { typeId });
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
      business: next.business,
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
  const save = async (d = draft) => {
    if (!d) throw "请先创建草稿";
    const saved = await invoke<Draft>("save_draft", {
      draft: draftSchema.parse(d),
    });
    setDraft(saved);
    setDirty(false);
    return saved;
  };
  const mutate = async (
    command: string,
    args: Record<string, unknown> = {},
  ) => {
    const d = await save();
    const next = await invoke<Draft>(command, { id: d.id, ...args });
    setDraft(next);
    setDirty(false);
    await refresh();
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
    await refresh();
  };
  const confirmLogin = async () => {
    let result: AuthResult;
    try {
      result = await invoke<AuthResult>("login", {
        token,
        remember: false,
      });
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
      setTab(tabForAuth(result));
      setError(result.message || "令牌校验失败，请重新输入。");
      setBoot((b) => (b ? { ...b, authorized: false, user: result.user } : b));
      return;
    }
    const next = await refresh();
    setBoot({ ...next, authorized: true, user: result.user });
    setTab("new");
    let validProject = false;
    try {
      validProject = await loadWorkspace(next.profile);
    } catch (e) {
      await handleRemoteError(e, "云效项目读取失败");
      return;
    }
    if (profileConfigured(next.profile) && validProject) {
      try {
        await loadProjectMetadataAndDraft(next.profile);
      } catch (e) {
        await handleRemoteError(e, "云效元数据读取失败");
        return;
      }
    }
    setTab("new");
    setNotice("登录成功，已进入新建提交。");
  };
  const issues =
    draft && boot
      ? validateDraft(draft, boot.profile, boot.metadata, boot.authorized)
      : [];
  const frozen = !!boot?.submissions.some((s) => s.draftId === draft?.id);
  const capturePending = (boot?.captures ?? []).some(
    (t) => t.draftId === draft?.id && t.status === "pending",
  );
  const currentSubmission = boot?.submissions.find(
    (s) => s.draftId === draft?.id,
  );
  const value = (path: string) =>
    path
      .split(".")
      .reduce<unknown>((v, p) => (v as Record<string, unknown>)?.[p], draft) ??
    "";
  const field = (
    path: string,
    label: string,
    opts: { type?: string; hint?: string; multi?: boolean } = {},
  ) => (
    <label className="field" id={`field-${path}`} key={path}>
      <span>
        {label}
        {draft?.sources[path] && (
          <small className={`source ${draft.sources[path].status}`}>
            {draft.sources[path].status === "collected"
              ? "自动采集"
              : draft.sources[path].status === "failed"
                ? "采集失败，待补"
                : "人工补充"}
          </small>
        )}
      </span>
      {opts.multi ? (
        <textarea
          value={String(value(path))}
          onChange={(e) => update(path, e.target.value)}
          rows={3}
        />
      ) : (
        <input
          type={opts.type ?? "text"}
          value={String(value(path))}
          onChange={(e) => update(path, e.target.value)}
          placeholder={opts.hint}
        />
      )}
      <small>
        {issues.find((i) => i.field === path)?.message ?? opts.hint}
      </small>
    </label>
  );
  const select = (
    path: string,
    label: string,
    options: Option[],
    multiple = false,
    optionError?: string,
  ) => (
    <label className="field" id={`field-${path}`} key={path}>
      <span>{label}</span>
      <select
        multiple={multiple}
        value={multiple ? (value(path) as string[]) : String(value(path))}
        onChange={(e) => {
          const nextValue = multiple
            ? Array.from(e.target.selectedOptions, (o) => o.value)
            : e.target.value;
          if (path === "workitemTypeId" && !multiple) {
            void run("读取类型字段", () => changeType(String(nextValue)));
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
      >
        {!multiple && <option value="">请选择</option>}
        {options.map((o) => (
          <option key={o.id} value={o.id}>
            {o.name}
            {(o as Option & { description?: string }).description
              ? ` · ${(o as Option & { description?: string }).description}`
              : ""}
          </option>
        ))}
      </select>
      <small>
        {issues.find((i) => i.field === path)?.message ??
          optionError ??
          (!options.length ? "API 暂未提供可选值" : "")}
      </small>
    </label>
  );
  const dynamicFields = boot?.metadata ? (
    <section className="card dynamic-fields">
      <div className="card-title">
        <h2>云效关键字段</h2>
        <span className="section-hint">选项来自当前项目</span>
      </div>
      <div className="grid">
        {boot.metadata.fields
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
          .filter((f) => f.showWhenCreate !== false || f.required)
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
                <p className="banner error" key={f.id}>
                  「{f.name}」格式 {f.format} 未支持，请完善映射或实现后再提交。
                </p>
              );
            if (
              [
                "list",
                "multiList",
                "user",
                "multiUser",
                "sprint",
                "version",
                "tag",
              ].includes(f.format)
            ) {
              return select(
                path,
                `${f.name}${f.required ? "（必填）" : ""}`,
                opts,
                ["multiList", "multiUser", "version", "tag"].includes(f.format),
                f.optionError,
              );
            }
            return field(path, `${f.name}${f.required ? "（必填）" : ""}`, {
              multi: f.format === "text",
              type: f.format === "date" ? "date" : "text",
            });
          })}
      </div>
      {!boot.metadata.fields.length && <p>当前类型没有额外字段。</p>}
    </section>
  ) : null;
  if (!isTauri())
    return (
      <main className="browser-message">
        <h1>Android Bug Desk</h1>
        <p>请启动桌面客户端。浏览器预览没有设备、凭据和本地文件权限。</p>
        <code>npm run dev</code>
      </main>
    );
  if (!boot)
    return (
      <main className="browser-message">
        <h1>打开登录页</h1>
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
          <div className="nav-caption">工作台</div>
          <nav>
            {[
              ["new", "01", "新建提交"],
              ["history", "02", "历史提交"],
            ].map(([id, num, title]) => (
              <button
                className={tab === id ? "active" : ""}
                onClick={() => setTab(id)}
                key={id}
              >
                <span>{num}</span>
                {title}
              </button>
            ))}
          </nav>
          <div className="aside-status">
            <UserProfile key={boot.user?.id || "anonymous"} user={boot.user} />
            <small>
              {boot.profile.name}
              <br />
              v0.1.0 · 本地优先
            </small>
          </div>
        </aside>
      )}
      <main>
        {boot.authorized && (
          <header>
            <div>
              <span className="eyebrow">Android 缺陷工作台</span>
              <h1>{tab === "history" ? "历史提交" : "新建提交"}</h1>
              <p>
                {tab === "history"
                  ? "查看本机持久记录和云效已创建的历史 Bug。"
                  : "连接设备，补齐 Bug 关键信息并提交可追溯证据。"}
              </p>
            </div>
            <div className="header-actions">
              <button
                disabled={
                  !profileConfigured(boot.profile) || !projectId || !!busy
                }
                title={!projectId ? "请先选择组织和项目" : undefined}
                onClick={() => void run("创建草稿", newDraft)}
              >
                ＋ 新建问题
              </button>
              {draft && (
                <span className="pill">
                  {frozen ? "证据已固定" : dirty ? "草稿未保存" : "草稿已保存"}
                </span>
              )}
              <button
                onClick={() =>
                  void run("退出登录", async () => {
                    if (dirty && draft) await save();
                    await memoryWrite.current;
                    workspaceEpoch.current += 1;
                    await invoke("clear_auth");
                    memoryWrite.current = Promise.resolve();
                    setSession(null);
                    setSerial("");
                    setDevices([]);
                    setPackageName("");
                    setBuildUrl("");
                    setBusiness("");
                    setDraft(null);
                    setDirty(false);
                    setAuth({
                      status: "missing",
                      user: null,
                      message: "未登录",
                    });
                    setOrganizations([]);
                    setProjects([]);
                    setRemoteHistory(null);
                    setBoot((b) =>
                      b
                        ? {
                            ...b,
                            authorized: false,
                            user: null,
                            metadata: null,
                          }
                        : b,
                    );
                    setTab("auth");
                  })
                }
              >
                退出登录
              </button>
            </div>
          </header>
        )}
        {boot.authorized && busy && (
          <div className="banner loading" role="status">
            {busy}…
          </div>
        )}
        {boot.authorized && error && (
          <div className="banner error" role="alert">
            {error}
          </div>
        )}
        {boot.authorized && notice && (
          <div className="banner success" role="status">
            {notice}
          </div>
        )}
        {tab === "auth" && (
          <section className="card login-card" aria-labelledby="login-title">
            <div className="login-brand">
              <span className="login-mark" aria-hidden="true">
                A
              </span>
              <span>Android Bug Desk</span>
            </div>
            <h2 id="login-title">登录云效</h2>
            <p className="login-intro">输入个人访问令牌，进入缺陷工作台。</p>
            {(error ||
              (auth &&
                auth.status !== "missing" &&
                auth.status !== "valid" &&
                auth.message)) && (
              <div className="banner error" role="alert">
                {error || auth?.message}
              </div>
            )}
            {busy && (
              <div className="banner loading" role="status">
                {busy}…
              </div>
            )}
            <form
              className="login-form"
              onSubmit={(event) => {
                event.preventDefault();
                void run("验证授权", confirmLogin);
              }}
            >
              <label className="field">
                <span>个人访问令牌</span>
                <input
                  autoComplete="off"
                  type="password"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="输入你的个人访问令牌"
                />
              </label>
              <button
                type="submit"
                className="primary"
                disabled={!token || !!busy}
              >
                确定
              </button>
            </form>
            <p className="login-note">令牌仅在本次会话中使用</p>
          </section>
        )}
        {tab === "new" && (
          <>
            <section className="card connection-card">
              <div className="card-title">
                <h2>USB 设备</h2>
                <button
                  onClick={() =>
                    void run("刷新设备", async () =>
                      setDevices(await invoke<Device[]>("list_devices")),
                    )
                  }
                >
                  刷新设备
                </button>
              </div>
              <p className="muted-text">
                {boot.adbPath
                  ? `已找到 ADB：${boot.adbPath}`
                  : "未检测到 Platform Tools。请选择 adb，或从 Android 官方网站安装。"}
              </p>
              <button
                onClick={() =>
                  void run("选择 ADB", async () => {
                    await invoke("choose_adb");
                    await refresh();
                  })
                }
              >
                选择 ADB 路径
              </button>
              <label className="field">
                <span>目标设备</span>
                <select
                  value={serial}
                  onChange={(e) => setSerial(e.target.value)}
                >
                  <option value="">明确选择一台设备</option>
                  {devices.map((d) => (
                    <option key={d.serial} value={d.serial}>
                      {d.label}
                    </option>
                  ))}
                </select>
                <small>
                  {devices.find((d) => d.serial === serial)?.state ===
                  "unauthorized"
                    ? "请在手机上允许 USB 调试授权"
                    : devices.find((d) => d.serial === serial)?.state ===
                        "offline"
                      ? "设备离线，请重新连接 USB"
                      : ""}
                </small>
              </label>
            </section>
            {tab === "new" && boot.authorized && (
              <section className="card project-selector">
                <div className="card-title">
                  <h2>云效项目</h2>
                  <div className="actions">
                    <span className="pill">当前账号可访问范围</span>
                    <button
                      disabled={!!busy}
                      onClick={() => void run("重试读取项目", retryWorkspace)}
                    >
                      刷新项目列表
                    </button>
                  </div>
                </div>
                <div className="grid">
                  <label className="field">
                    <span>组织</span>
                    <select
                      value={organizationId}
                      disabled={!!busy}
                      onChange={(e) =>
                        void run("读取项目", async () => {
                          const id = e.target.value;
                          setOrganizationId(id);
                          setProjectId("");
                          try {
                            setProjects(
                              id
                                ? await invoke<Option[]>("list_projects", {
                                    organizationId: id,
                                  })
                                : [],
                            );
                          } catch (error) {
                            await handleRemoteError(error, "云效项目读取失败");
                          }
                        })
                      }
                    >
                      <option value="">请选择组织</option>
                      {organizations.map((o) => (
                        <option key={o.id} value={o.id}>
                          {o.name}
                        </option>
                      ))}
                    </select>
                    {!organizations.length && (
                      <small>API 暂未返回可用组织。</small>
                    )}
                  </label>
                  <label className="field">
                    <span>项目</span>
                    <select
                      value={projectId}
                      disabled={!organizationId || !!busy}
                      onChange={(e) =>
                        void run("切换项目", async () => {
                          try {
                            await selectProject(organizationId, e.target.value);
                          } catch (error) {
                            await handleRemoteError(error, "云效项目切换失败");
                          }
                        })
                      }
                    >
                      <option value="">请选择项目</option>
                      {projects.map((o) => (
                        <option key={o.id} value={o.id}>
                          {o.name}
                        </option>
                      ))}
                    </select>
                    {organizationId && !projects.length && (
                      <small>API 暂未返回该组织的可用项目。</small>
                    )}
                  </label>
                </div>
                <p className="muted-text">
                  当前项目：{boot.profile.name}
                  。切换项目会建立新的当前草稿，旧草稿和提交记录保留。
                </p>
              </section>
            )}
            <details className="card" open={!!session}>
              <summary>设备采集与固定构建</summary>
              <h2>测试会话</h2>
              <div className="grid">
                <label className="field">
                  <span>业务方向</span>
                  <select
                    value={business}
                    onChange={(e) => setBusiness(e.target.value)}
                  >
                    <option value="">请选择</option>
                    {boot.profile.businessProfiles.map((p) => (
                      <option key={p}>{p}</option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>目标包名</span>
                  <input
                    value={packageName}
                    onChange={(e) => setPackageName(e.target.value)}
                    placeholder="例如 com.example.app"
                  />
                </label>
                <label className="field wide">
                  <span>Jenkins 固定构建 URL</span>
                  <input
                    value={buildUrl}
                    onChange={(e) => setBuildUrl(e.target.value)}
                    placeholder="https://jenkins.example/job/App/123/"
                  />
                </label>
              </div>
              <div className="actions">
                <button
                  className="primary"
                  disabled={!serial || !packageName || !business || !!busy}
                  onClick={() =>
                    void run("开始采集", async () => {
                      const s = await invoke<Session>("start_session", {
                        serial,
                        packageName,
                        buildUrl,
                        business,
                      });
                      setSession(s);
                      setNotice(
                        "正在记录 main / system / crash 日志。复现后点击“记录问题”。",
                      );
                    })
                  }
                >
                  开始采集
                </button>
                <button
                  disabled={!session}
                  onClick={() =>
                    void run("停止采集", async () => {
                      await invoke("stop_session");
                      setSession(null);
                      await refresh();
                      setNotice("会话已结束，日志已保留。");
                    })
                  }
                >
                  停止采集
                </button>
                <button
                  disabled={!session || frozen || capturePending || !!busy}
                  onClick={() =>
                    void run("记录问题", async () => {
                      let d = draft;
                      if (!d) d = await invoke<Draft>("new_draft");
                      else if (dirty) d = await save();
                      setDraft(
                        await invoke<Draft>("mark_problem", { id: d.id }),
                      );
                      setDirty(false);
                      setTab("new");
                      await refresh();
                    })
                  }
                >
                  记录问题并截图
                </button>
              </div>
              {session && (
                <div className="session-live">
                  <span className="dot" />
                  正在采集 · {session.serial} · {session.packageName}
                </div>
              )}
              <p className="muted-text">
                开始新会话会结束原会话。断线不会自动切换设备。记录问题后保持采集，
                30 秒后自动固定日志；提前结束会如实记录缺口。
              </p>
            </details>
          </>
        )}
        {tab === "new" &&
          (!draft ? (
            <section className="card empty">
              <h2>记录第一个问题</h2>
              <p>请选择云效项目，创建草稿后连接设备采集或导入已有证据。</p>
              <button
                className="primary"
                onClick={() => void run("创建草稿", newDraft)}
              >
                新建问题草稿
              </button>
            </section>
          ) : (
            <>
              <div className="draft-switch">
                <select
                  value={draft.id}
                  onChange={(e) =>
                    void run("切换草稿", async () => {
                      if (dirty && !frozen) await save();
                      setDraft(
                        boot.drafts.find((d) => d.id === e.target.value) ??
                          null,
                      );
                      setDirty(false);
                      setPreview(false);
                    })
                  }
                >
                  {boot.drafts.map((d) => (
                    <option value={d.id} key={d.id}>
                      {d.title || "未命名草稿"} ·{" "}
                      {new Date(d.createdAt).toLocaleString()}
                    </option>
                  ))}
                </select>
                <small>
                  {draft.title || "未命名草稿"} ·{" "}
                  {new Date(draft.createdAt).toLocaleString()}
                </small>
              </div>
              {frozen && (
                <div className="banner">
                  {stateLabels[currentSubmission?.state ?? "ready"]}
                  。证据已固定，可在提交记录中核对或补传。
                </div>
              )}
              <fieldset disabled={frozen || !!busy}>
                <section className="card">
                  <h2>问题与范围</h2>
                  <div className="grid">
                    {field("title", "具体问题标题", {
                      hint: "写清在什么操作下发生什么行为",
                    })}
                    {field("actual", "问题描述 / 实际结果", { multi: true })}
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
                        "性能",
                        "网络",
                        "数据",
                      ].map((v) => ({ id: v, name: v })),
                    )}
                    {select(
                      "business",
                      "业务方向",
                      boot.profile.businessProfiles.map((p) => ({
                        id: p,
                        name: p,
                      })),
                    )}
                    {select(
                      "workitemTypeId",
                      "云效 Bug 类型",
                      boot.metadata?.types ?? [],
                    )}
                  </div>
                  <button
                    disabled={!boot.authorized}
                    onClick={() =>
                      void run("刷新类型字段", async () => {
                        await save();
                        await invoke("load_metadata", {
                          typeId: draft.workitemTypeId,
                        });
                        await refresh();
                      })
                    }
                  >
                    读取此类型的最新字段
                  </button>
                  <div className="grid">
                    {select(
                      "assignedTo",
                      "负责人",
                      boot.metadata?.members ?? [],
                    )}
                    {select(
                      "sprint",
                      "迭代（版本或迭代至少一项）",
                      boot.metadata?.sprints ?? [],
                    )}
                    {select(
                      "versions",
                      "版本（可多选）",
                      boot.metadata?.versions ?? [],
                      true,
                    )}
                    {field("buildUrl", "固定 Jenkins 构建 URL")}
                  </div>
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={draft.buildStatus === "manual_confirmed"}
                      onChange={(e) =>
                        update(
                          "buildStatus",
                          e.target.checked ? "manual_confirmed" : "unconfirmed",
                        )
                      }
                    />
                    我已确认此固定构建对应当前测试安装包（人工确认）
                  </label>
                  <p className="muted-text">
                    精确产物与 mapping 匹配仍需构建清单和安装指纹验证。
                  </p>
                </section>
                {dynamicFields}
                <section className="card">
                  <h2>应用与设备</h2>
                  <div className="grid">
                    {field("app.packageName", "包名")}
                    {field("app.versionName", "versionName")}
                    {field("app.versionCode", "versionCode（允许 0）")}
                    {field("app.channel", "渠道")}
                    {field("app.environment", "测试环境")}
                    {field("app.obfuscation", "混淆状态", {
                      hint: "例如 已混淆 / 未混淆，注明依据",
                    })}
                    {field("app.hardening", "加固状态", {
                      hint: "例如 已加固 / 未加固，注明依据",
                    })}
                    {field("device.model", "设备型号")}
                    {field("device.androidVersion", "Android 版本")}
                    {field("device.totalMemKb", "总内存（kB）")}
                    {field("device.availableMemKb", "当前可用内存（kB）")}
                    {field("device.resolution", "分辨率（像素）", {
                      hint: "1080x2400",
                    })}
                    {field("device.density", "屏幕密度（dpi）")}
                    {field("manualNote", "人工补充信息来源", {
                      hint: "例如 设备设置截图 / 应用详情页面",
                    })}
                  </div>
                </section>
                <section className="card">
                  <h2>复现事实</h2>
                  <div className="grid">
                    {field("event.occurredAt", "真实问题时间（含时区）", {
                      hint: "2026-10-09T14:30:00+08:00",
                    })}
                    {field("event.sourceNote", "事件时间来源 / 修改依据")}
                    {field("preconditions", "前置条件", {
                      multi: true,
                      hint: "安装方式、清数据、账号、权限与网络状态",
                    })}
                    {field("steps", "复现步骤（每行一步）", { multi: true })}
                    {field("expected", "预期结果", { multi: true })}
                  </div>
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={draft.event.lateCapture}
                      onChange={(e) =>
                        update("event.lateCapture", e.target.checked)
                      }
                    />
                    问题发生后才开始采集（事后补采）
                  </label>
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
                    <div className="grid">
                      {field("frequency.attempts", "尝试次数")}
                      {field("frequency.occurrences", "发生次数")}
                    </div>
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
                </section>
                <section className="card">
                  <h2>设备有效云控</h2>
                  {select("cloud.applicability", "此功能是否有云控", [
                    { id: "yes", name: "适用：填写设备实际生效证据" },
                    { id: "no", name: "不适用：填写功能依据" },
                  ])}
                  {draft.cloud.applicability === "no"
                    ? field("cloud.reason", "该功能无云控的依据", {
                        multi: true,
                      })
                    : draft.cloud.applicability === "yes" && (
                        <div className="grid">
                          {field("cloud.name", "配置 / 实验名称")}
                          {field("cloud.effectiveValue", "设备实际生效值")}
                          {select(
                            "cloud.source",
                            "判断来源",
                            [
                              "应用调试面板",
                              "应用导出",
                              "相关运行日志",
                              "用户可观察行为",
                            ].map((v) => ({ id: v, name: v })),
                          )}
                          {field("cloud.observedAt", "观察时间（含时区）")}
                          {field("cloud.override", "测试覆盖值声明", {
                            hint: "例如 未设置覆盖 / 覆盖为 true",
                          })}
                        </div>
                      )}
                </section>
              </fieldset>
              <section className="card">
                <div className="card-title">
                  <h2>
                    证据附件 <small>{draft.attachments.length} 份</small>
                  </h2>
                  <span className="pill">逐份预览并确认上传</span>
                </div>
                <div className="actions">
                  {[
                    ["log", "导入日志"],
                    ["image", "导入截图"],
                    ["video", "导入 MP4"],
                    ["report", "导入系统报告 ZIP"],
                    ["sample", "导入数据样例"],
                  ].map(([role, label]) => (
                    <button
                      disabled={frozen || !!busy}
                      key={role}
                      onClick={() =>
                        void run(label, () =>
                          mutate("import_attachment", { role }),
                        )
                      }
                    >
                      {label}
                    </button>
                  ))}
                  <button
                    disabled={frozen || !draft.sessionId || !!busy}
                    onClick={() =>
                      void run("固定日志", () => mutate("pin_logs"))
                    }
                  >
                    恢复中断窗口记录
                  </button>
                </div>
                <div className="actions">
                  <button
                    disabled={frozen || !draft.sessionId || !!busy}
                    onClick={() =>
                      void run("采集系统报告（最长两分钟）", () =>
                        mutate("capture_bugreport"),
                      )
                    }
                  >
                    采集系统报告
                  </button>
                </div>
                {(boot.captures ?? [])
                  .filter((t) => t.draftId === draft.id)
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
                <p className="muted-text">
                  派生副本与来源回执一起选择；移除副本会同时移除关联回执，回执不能单独移除。
                  文件先保存为本地副本。勾选后才进入证据包和上传；日志请填真实覆盖范围。超过{" "}
                  {boot.profile.attachmentWarningMb} MB
                  为本地提醒，不代表云效限制。
                </p>
                {draft.attachments.map((a) => (
                  <AttachmentCard
                    key={a.id}
                    attachment={a}
                    disabled={frozen || !!busy}
                    onConfirm={(confirmed, start, end) =>
                      void run("保存附件确认", () =>
                        mutate("confirm_attachment", {
                          attachmentId: a.id,
                          confirmed,
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
                    onRead={() =>
                      void run("预览日志", async () =>
                        setLog(
                          await invoke<string>("read_log", {
                            id: draft.id,
                            attachmentId: a.id,
                          }),
                        ),
                      )
                    }
                  />
                ))}
              </section>
              <section className="card">
                <h2>提交前核对</h2>
                {issues.filter((i) => i.field !== "previewConfirmed").length >
                0 ? (
                  <div className="missing-list">
                    {issues
                      .filter((i) => i.field !== "previewConfirmed")
                      .map((i, n) => (
                        <button
                          key={n}
                          onClick={() =>
                            document
                              .getElementById(`field-${i.field}`)
                              ?.scrollIntoView({
                                behavior: "smooth",
                                block: "center",
                              })
                          }
                        >
                          {i.message}
                        </button>
                      ))}
                  </div>
                ) : (
                  <p className="success-text">基础信息已齐全，可以进入预览。</p>
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
                    disabled={frozen || !!busy}
                    onClick={() =>
                      void run("保存草稿", async () => {
                        await save();
                        await refresh();
                        setNotice("草稿已保存在本机。");
                      })
                    }
                  >
                    保存草稿
                  </button>
                  <button
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
                  </button>
                  <button
                    className="primary"
                    disabled={
                      frozen ||
                      !!busy ||
                      capturePending ||
                      issues.some((i) => i.field !== "previewConfirmed")
                    }
                    onClick={() =>
                      void run("核对主进程校验", async () => {
                        const d = await save();
                        const errors = await invoke<
                          { field: string; message: string }[]
                        >("validate_draft", { id: d.id });
                        if (errors.some((i) => i.field !== "previewConfirmed"))
                          throw errors.map((i) => i.message).join("；");
                        setPreview(true);
                      })
                    }
                  >
                    进入提交预览
                  </button>
                </div>
              </section>
              {preview && (
                <section className="card preview">
                  <h2>提交预览</h2>
                  <dl>
                    <dt>云效项目</dt>
                    <dd>{boot.profile.name}</dd>
                    <dt>负责人</dt>
                    <dd>
                      {boot.metadata?.members.find(
                        (m) => m.id === draft.assignedTo,
                      )?.name ?? "未选择"}
                    </dd>
                    <dt>业务与范围</dt>
                    <dd>
                      {draft.business} · 版本 {draft.versions.join(", ")} · 迭代{" "}
                      {draft.sprint}
                    </dd>
                    <dt>构建</dt>
                    <dd>{draft.buildUrl}</dd>
                    <dt>标题</dt>
                    <dd>{draft.title}</dd>
                    <dt>前置条件</dt>
                    <dd>{draft.preconditions}</dd>
                    <dt>步骤</dt>
                    <dd className="preserve">{draft.steps}</dd>
                    <dt>预期 / 实际</dt>
                    <dd>
                      {draft.expected}
                      <hr />
                      {draft.actual}
                    </dd>
                    <dt>设备 / 应用 / 时间 / 云控</dt>
                    <dd className="preserve">
                      {JSON.stringify(
                        {
                          device: draft.device,
                          app: draft.app,
                          event: draft.event,
                          cloud: draft.cloud,
                        },
                        null,
                        2,
                      )}
                    </dd>
                    <dt>将上传</dt>
                    <dd>
                      {draft.attachments
                        .filter((a) => a.confirmed)
                        .map((a) => a.name)
                        .join("、")}
                      ，以及不可变证据 ZIP
                    </dd>
                  </dl>
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={draft.previewConfirmed}
                      onChange={(e) => {
                        const next = {
                          ...draft,
                          previewConfirmed: e.target.checked,
                        };
                        setDraft(next);
                        setDirty(true);
                      }}
                    />
                    我已核对项目、构建、描述与附件内容，并确认上传
                  </label>
                  <button
                    className="primary"
                    disabled={
                      !draft.previewConfirmed || !!busy || capturePending
                    }
                    onClick={() =>
                      void run("提交 Bug 并核对附件", async () => {
                        const d = await save();
                        const s = await invoke<Submission>("submit_bug", {
                          id: d.id,
                        });
                        await refresh();
                        setTab("history");
                        setNotice(stateLabels[s.state] ?? s.state);
                      })
                    }
                  >
                    提交 Bug
                  </button>
                </section>
              )}
            </>
          ))}
        {tab === "history" && (
          <>
            <section className="card">
              <div className="card-title">
                <h2>本机提交记录</h2>
                <button onClick={() => void run("刷新记录", refresh)}>
                  刷新
                </button>
              </div>
              {!boot.submissions.length ? (
                <div className="empty">
                  <p>还没有本机提交记录。</p>
                </div>
              ) : (
                boot.submissions.map((s) => (
                  <article className="submission" key={s.draftId}>
                    <div className="card-title">
                      <h3>
                        {boot.drafts.find((d) => d.id === s.draftId)?.title}
                      </h3>
                      <span className="pill">
                        {stateLabels[s.state] ?? s.state}
                      </span>
                    </div>
                    <p>
                      工作项 {s.identifier || s.workitemId || "尚未获得确定 ID"}{" "}
                      ·{" "}
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
                ))
              )}
            </section>
            <section className="card">
              <div className="card-title">
                <h2>云效历史 Bug</h2>
                <button
                  disabled={!!busy}
                  onClick={() => {
                    setRemoteHistoryError("");
                    void run("读取云效历史", async () => {
                      setRemoteHistory(
                        await invoke<HistoryPage>("list_remote_history", {
                          page: 1,
                        }),
                      );
                    });
                  }}
                >
                  刷新
                </button>
              </div>
              {remoteHistoryError ? (
                <p className="banner error">
                  云效历史读取失败：{remoteHistoryError}
                </p>
              ) : remoteHistory ? (
                remoteHistory.items.length ? (
                  remoteHistory.items.map((item) => (
                    <article className="submission" key={item.id}>
                      <div className="card-title">
                        <h3>{item.title}</h3>
                        <span className="pill">{item.status}</span>
                      </div>
                      <p>
                        工作项 {item.identifier || item.id} · 创建于{" "}
                        {item.createdAt}
                      </p>
                    </article>
                  ))
                ) : (
                  <div className="empty">
                    <p>云效没有返回当前用户在此项目中的历史 Bug。</p>
                  </div>
                )
              ) : (
                <p className="muted-text">
                  正在读取云效历史；读取失败会明确提示，不会伪装为空记录。
                </p>
              )}
              {remoteHistory?.hasMore && (
                <button
                  disabled={!!busy}
                  onClick={() =>
                    void run("加载更多历史", async () => {
                      const nextPage = remoteHistoryPage + 1;
                      const page = await invoke<HistoryPage>(
                        "list_remote_history",
                        { page: nextPage },
                      );
                      setRemoteHistory((current) =>
                        current
                          ? {
                              ...page,
                              items: [
                                ...current.items,
                                ...page.items.filter(
                                  (item) =>
                                    !current.items.some(
                                      (old) => old.id === item.id,
                                    ),
                                ),
                              ],
                              page: nextPage,
                            }
                          : page,
                      );
                      setRemoteHistoryPage(nextPage);
                    })
                  }
                >
                  加载更多
                </button>
              )}
            </section>
          </>
        )}
        {log && (
          <div className="modal">
            <section>
              <div className="card-title">
                <h2>日志预览（前 100 KB）</h2>
                <button onClick={() => setLog("")}>关闭</button>
              </div>
              <p>
                请确认账号、路径、网络及业务信息。预览不表示所有敏感数据均已消除。
              </p>
              <pre>{log}</pre>
            </section>
          </div>
        )}
      </main>
    </div>
  );
}
function AttachmentCard({
  attachment: a,
  disabled,
  onConfirm,
  onRemove,
  onRead,
  onRedact,
}: {
  attachment: Attachment;
  disabled: boolean;
  onConfirm: (c: boolean, s: string, e: string) => void;
  onRemove: () => void;
  onRead: () => void;
  onRedact: (literal: string) => void;
}) {
  const [start, setStart] = useState(a.coverageStart),
    [end, setEnd] = useState(a.coverageEnd);
  const [literal, setLiteral] = useState("");
  return (
    <article className="attachment">
      <div className="attachment-body">
        {a.role === "image" && (
          <img src={convertFileSrc(a.path)} alt={a.name} />
        )}{" "}
        {a.role === "video" && (
          <video controls preload="metadata" src={convertFileSrc(a.path)} />
        )}
        <div>
          <strong>{a.name}</strong>
          <small>
            {a.role} · {(a.size / 1024 / 1024).toFixed(2)} MB · {a.source}
          </small>
          <code>SHA256 {a.sha256}</code>
          {a.role === "log" && (
            <>
              <button onClick={onRead}>预览日志</button>
              <details className="redaction-control">
                <summary>生成脱敏副本</summary>
                <p>
                  指定需要替换的文字，原文件保留。生成后须重新预览并确认副本；只替换完全匹配的文字。
                </p>
                <label className="field">
                  <span>需要替换的文字</span>
                  <input
                    type="password"
                    autoComplete="off"
                    disabled={disabled}
                    value={literal}
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
                <label className="field">
                  <span>真实覆盖起点（含时区）</span>
                  <input
                    disabled={disabled}
                    value={start}
                    onChange={(e) => setStart(e.target.value)}
                  />
                </label>
                <label className="field">
                  <span>真实覆盖终点（含时区）</span>
                  <input
                    disabled={disabled}
                    value={end}
                    onChange={(e) => setEnd(e.target.value)}
                  />
                </label>
              </div>
            </>
          )}
        </div>
      </div>
      <div className="actions">
        <label className="check">
          <input
            disabled={disabled}
            type="checkbox"
            checked={a.confirmed}
            onChange={(e) => onConfirm(e.target.checked, start, end)}
          />
          已预览，确认上传
        </label>
        {a.role === "log" && (
          <button
            disabled={disabled}
            onClick={() => onConfirm(a.confirmed, start, end)}
          >
            保存覆盖时间
          </button>
        )}
        <button disabled={disabled} onClick={onRemove}>
          从草稿移除
        </button>
      </div>
    </article>
  );
}
createRoot(document.getElementById("root")!).render(<App />);

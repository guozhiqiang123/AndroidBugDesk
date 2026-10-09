import React, { useEffect, useState } from "react";
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
import {
  nativeTarget,
  validateDraft,
  supportedFormats,
} from "../shared/validation";
import "./style.css";

interface Bootstrap {
  profile: Profile;
  authorized: boolean;
  metadata: Metadata | null;
  adbPath: string | null;
  drafts: Draft[];
  submissions: Submission[];
  sessions: Session[];
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
  const [boot, setBoot] = useState<Bootstrap | null>(null),
    [draft, setDraft] = useState<Draft | null>(null);
  const [tab, setTab] = useState("auth"),
    [busy, setBusy] = useState(""),
    [notice, setNotice] = useState(""),
    [error, setError] = useState("");
  const [token, setToken] = useState(""),
    [remember, setRemember] = useState(false),
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
  const refresh = async () => {
    const b = await invoke<Bootstrap>("bootstrap");
    setBoot(b);
    return b;
  };
  const run = async (name: string, work: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(name);
    setError("");
    setNotice("");
    try {
      await work();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy("");
    }
  };
  useEffect(() => {
    if (isTauri())
      void run("加载本地草稿", async () => {
        const b = await refresh();
        setDraft(b.drafts[0] ?? null);
        setTab(b.authorized ? "capture" : "auth");
      });
  }, []);
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
    if (dirty && draft) await save();
    const d = await invoke<Draft>("new_draft");
    setDraft(d);
    setDirty(false);
    setPreview(false);
    setTab("problem");
    await refresh();
  };
  const issues =
    draft && boot
      ? validateDraft(draft, boot.profile, boot.metadata, boot.authorized)
      : [];
  const frozen = !!boot?.submissions.some((s) => s.draftId === draft?.id);
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
  ) => (
    <label className="field" id={`field-${path}`} key={path}>
      <span>{label}</span>
      <select
        multiple={multiple}
        value={multiple ? (value(path) as string[]) : String(value(path))}
        onChange={(e) =>
          update(
            path,
            multiple
              ? Array.from(e.target.selectedOptions, (o) => o.value)
              : e.target.value,
          )
        }
      >
        {!multiple && <option value="">请选择</option>}
        {options.map((o) => (
          <option key={o.id} value={o.id}>
            {o.name}
            {o.name !== o.id ? ` · ${o.id}` : ""}
          </option>
        ))}
      </select>
      <small>{issues.find((i) => i.field === path)?.message}</small>
    </label>
  );
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
        <h1>正在打开工作台</h1>
        <p>{error || "读取本地草稿…"}</p>
      </main>
    );
  return (
    <div className="app">
      <aside>
        <div className="brand">
          <span className="brand-icon">A</span>
          <div>
            Android Bug Desk<small>测试证据工作台</small>
          </div>
        </div>
        <div className="nav-caption">提交一个可追溯的问题</div>
        <nav>
          {[
            ["auth", "01", "授权与项目"],
            ["capture", "02", "连接与采集"],
            ["problem", "03", "问题与证据"],
            ["history", "04", "提交记录"],
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
          <span className={boot.authorized ? "dot" : "dot muted"} />
          {boot.authorized ? "个人授权已载入" : "可离线采集与保存"}
          <small>
            {boot.profile.name}
            <br />
            v0.1.0 · 本地优先
          </small>
        </div>
      </aside>
      <main>
        <header>
          <div>
            <span className="eyebrow">ANDROID QA WORKSPACE</span>
            <h1>
              {
                {
                  auth: "授权与项目",
                  capture: "连接与采集",
                  problem: "问题与证据",
                  history: "提交记录",
                }[tab]
              }
            </h1>
            <p>
              {
                {
                  auth: "使用你的个人权限提交，草稿始终保存在本机。",
                  capture: "明确选择 USB 设备，让日志跟随每一次复现。",
                  problem: "补齐事实、确认材料，再把完整证据交给开发。",
                  history: "查看真实提交阶段，恢复尚未完成的证据上传。",
                }[tab]
              }
            </p>
          </div>
          <div className="header-actions">
            <button onClick={() => void run("创建草稿", newDraft)}>
              ＋ 新建问题
            </button>
            {draft && (
              <span className="pill">
                {frozen ? "证据已固定" : dirty ? "草稿未保存" : "草稿已保存"}
              </span>
            )}
          </div>
        </header>
        {busy && (
          <div className="banner loading" role="status">
            {busy}…
          </div>
        )}
        {error && (
          <div className="banner error" role="alert">
            {error}
          </div>
        )}
        {notice && (
          <div className="banner success" role="status">
            {notice}
          </div>
        )}
        {tab === "auth" && (
          <>
            <section className="card">
              <div className="card-title">
                <h2>个人云效授权</h2>
                <span className="pill">
                  {boot.metadata ? "项目读取已验证" : "未完成项目读取验证"}
                </span>
              </div>
              <p className="muted-text">
                请输入个人访问令牌。只读验证不会创建缺陷，也不代表附件写入权限已验收。
              </p>
              <label className="field">
                <span>个人 PAT</span>
                <input
                  autoComplete="off"
                  type="password"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="输入你的个人访问令牌"
                />
              </label>
              <label className="check">
                <input
                  type="checkbox"
                  checked={remember}
                  onChange={(e) => setRemember(e.target.checked)}
                />
                在系统安全存储中保存（钥匙串 / Windows 凭据 / Linux Secret
                Service）
              </label>
              <div className="actions">
                <button
                  className="primary"
                  disabled={!token || !!busy}
                  onClick={() =>
                    void run("验证授权", async () => {
                      await invoke("set_auth", { token, remember });
                      setToken("");
                      await refresh();
                      const m = await invoke<Metadata>("load_metadata", {
                        typeId: "",
                      });
                      setBoot((b) => (b ? { ...b, metadata: m } : b));
                      setNotice(
                        "项目读取已验证；真实创建和媒体上传仍需单独验收。",
                      );
                    })
                  }
                >
                  授权并读取项目
                </button>
                <button
                  onClick={() =>
                    void run("清除授权", async () => {
                      await invoke("clear_auth");
                      setToken("");
                      await refresh();
                    })
                  }
                >
                  清除授权
                </button>
              </div>
            </section>
            <section className="card">
              <h2>项目配置</h2>
              <div className="facts">
                <div>
                  <small>项目</small>
                  <strong>{boot.profile.name}</strong>
                </div>
                <div>
                  <small>组织 ID</small>
                  <code>{boot.profile.organizationId}</code>
                </div>
                <div>
                  <small>项目 ID</small>
                  <code>{boot.profile.projectId}</code>
                </div>
              </div>
              <p>{boot.profile.businessProfiles.join(" / ")}</p>
              <div className="actions">
                <button
                  onClick={() =>
                    void run("导入配置", async () => {
                      await invoke("import_profile");
                      await refresh();
                    })
                  }
                >
                  导入项目 JSON
                </button>
                <button
                  disabled={!boot.authorized}
                  onClick={() =>
                    void run("读取项目配置", async () => {
                      await invoke("load_metadata", {
                        typeId: draft?.workitemTypeId ?? "",
                      });
                      await refresh();
                    })
                  }
                >
                  重新读取配置
                </button>
              </div>
            </section>
          </>
        )}
        {tab === "capture" && (
          <>
            <section className="card">
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
            <section className="card">
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
                  disabled={!session || frozen}
                  onClick={() =>
                    void run("记录问题", async () => {
                      let d = draft;
                      if (!d) d = await invoke<Draft>("new_draft");
                      else if (dirty) d = await save();
                      setDraft(
                        await invoke<Draft>("mark_problem", { id: d.id }),
                      );
                      setDirty(false);
                      setTab("problem");
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
                  <small>会话 {session.id}</small>
                </div>
              )}
              <p className="muted-text">
                开始新会话会结束原会话。断线不会自动切换设备。记录问题后建议继续约
                30 秒，再固定日志证据。
              </p>
            </section>
          </>
        )}
        {tab === "problem" &&
          (!draft ? (
            <section className="card empty">
              <h2>记录第一个问题</h2>
              <p>可以先连接设备自动采集，也可以创建离线草稿并导入已有证据。</p>
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
                      {d.title || "未命名问题"} · {d.id.slice(0, 8)}
                    </option>
                  ))}
                </select>
                <code>{draft.id}</code>
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
                    {field("actual", "实际结果", { multi: true })}
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
                {boot.metadata && (
                  <section className="card">
                    <h2>云效项目字段</h2>
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
                      .map((f) => {
                        const path = `customFields.${f.id}`;
                        const opts = ["user", "multiUser"].includes(f.format)
                          ? boot.metadata!.members
                          : f.format === "sprint"
                            ? boot.metadata!.sprints
                            : f.format === "version"
                              ? boot.metadata!.versions
                              : f.options;
                        return !supportedFormats.has(f.format) ? (
                          <p className="banner error" key={f.id}>
                            「{f.name}」格式 {f.format}{" "}
                            未支持，请完善映射或实现后再提交。
                          </p>
                        ) : [
                            "list",
                            "multiList",
                            "user",
                            "multiUser",
                            "sprint",
                            "version",
                          ].includes(f.format) ? (
                          select(
                            path,
                            `${f.name}${f.required ? "（必填）" : ""}`,
                            opts,
                            ["multiList", "multiUser", "version"].includes(
                              f.format,
                            ),
                          )
                        ) : (
                          field(
                            path,
                            `${f.name}${f.required ? "（必填）" : ""}`,
                            {
                              multi: f.format === "text",
                              type: f.format === "date" ? "date" : "text",
                            },
                          )
                        );
                      })}
                    {!boot.metadata.fields.length && (
                      <p>当前类型没有额外字段。</p>
                    )}
                  </section>
                )}
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
                    固定会话日志
                  </button>
                </div>
                <p className="muted-text">
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
                    <dd>
                      {boot.profile.name} · {boot.profile.projectId}
                    </dd>
                    <dt>负责人</dt>
                    <dd>
                      {
                        boot.metadata?.members.find(
                          (m) => m.id === draft.assignedTo,
                        )?.name
                      }{" "}
                      · {draft.assignedTo}
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
                    disabled={!draft.previewConfirmed || !!busy}
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
          <section className="card">
            <div className="card-title">
              <h2>持久提交记录</h2>
              <button onClick={() => void run("刷新记录", refresh)}>
                刷新
              </button>
            </div>
            {!boot.submissions.length ? (
              <div className="empty">
                <p>还没有提交记录。离线草稿保留在“问题与证据”。</p>
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
              ))
            )}
          </section>
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
}: {
  attachment: Attachment;
  disabled: boolean;
  onConfirm: (c: boolean, s: string, e: string) => void;
  onRemove: () => void;
  onRead: () => void;
}) {
  const [start, setStart] = useState(a.coverageStart),
    [end, setEnd] = useState(a.coverageEnd);
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

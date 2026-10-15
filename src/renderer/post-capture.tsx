import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Draft } from "../shared/domain";
import {
  captureItems,
  captureStatusLabels,
  latestCaptureResults,
  recommendedCaptureItems,
  recommendedUncollectedItems,
  retryableItems,
  type CaptureItemId,
  type PostCaptureJob,
  type PostCaptureSnapshot,
} from "../shared/post-capture";
import { AppDialog } from "./app-dialog";
import "./post-capture.css";

export function PostCaptureEntry({
  draft,
  serial,
  disabled,
  onPrepare,
  onUpdate,
  onComplete,
}: {
  draft: Draft;
  serial: string;
  disabled: boolean;
  onPrepare: () => Promise<unknown>;
  onUpdate: (snapshot: PostCaptureSnapshot) => void;
  onComplete: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<CaptureItemId[]>(() =>
    recommendedCaptureItems(draft.kind),
  );
  const [jobs, setJobs] = useState<PostCaptureJob[]>([]);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState("");
  const [clock, setClock] = useState(() => Date.now());
  const alive = useRef(true);
  const update = useRef(onUpdate);
  const complete = useRef(onComplete);
  const pendingJobs = useRef(new Set<string>());
  update.current = onUpdate;
  complete.current = onComplete;
  const running = jobs.some((job) => job.status === "pending");
  const latest = jobs[0];
  const load = async () => {
    const snapshot = await invoke<PostCaptureSnapshot>("post_capture_status", {
      id: draft.id,
    });
    if (!alive.current) return [];
    const visible = snapshot.jobs.filter(
      (job) => !(Object.keys(job.items).length === 1 && job.items.environment),
    );
    for (const job of visible) {
      if (job.status === "pending") {
        pendingJobs.current.add(job.id);
      } else if (
        pendingJobs.current.delete(job.id) &&
        job.status === "complete"
      ) {
        setOpen(false);
        complete.current();
      }
    }
    setJobs(visible);
    if (!visible.some((job) => job.status === "pending")) {
      const complete = new Set(
        latestCaptureResults(visible)
          .filter((entry) => entry.result.status === "complete")
          .map((entry) => entry.itemId),
      );
      setSelected((current) => current.filter((item) => !complete.has(item)));
    }
    update.current(snapshot);
    return visible;
  };
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (!running) return;
    let active = true,
      polling = false;
    const timer = window.setInterval(() => {
      if (polling) return;
      setClock(Date.now());
      polling = true;
      void load()
        .catch((failure) => {
          if (active && alive.current)
            setError(`读取补采进度失败：${String(failure)}`);
        })
        .finally(() => {
          polling = false;
        });
    }, 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [running, draft.id]);
  const begin = async (items: CaptureItemId[], retryOf?: string) => {
    if (starting || running || disabled || !serial || !items.length) return;
    setStarting(true);
    setError("");
    try {
      await onPrepare();
      if (!alive.current) return;
      const job = await invoke<PostCaptureJob>("start_post_capture", {
        id: draft.id,
        serial,
        items,
        retryOf: retryOf ?? null,
      });
      if (!alive.current) return;
      setJobs((current) => [job, ...current]);
      await load();
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      if (alive.current) setStarting(false);
    }
  };
  const show = async () => {
    setError("");
    setOpen(true);
    try {
      const visible = await load();
      if (alive.current)
        setSelected(recommendedUncollectedItems(draft.kind, visible));
    } catch (failure) {
      if (alive.current) setError(String(failure));
    }
  };
  const latestResults = latestCaptureResults(jobs);
  const completedItems = new Set(
    latestResults
      .filter((entry) => entry.result.status === "complete")
      .map((entry) => entry.itemId),
  );
  const progressText = (itemId: CaptureItemId, startedAt?: string) => {
    if (itemId !== "bugreport" || !startedAt) return "";
    const started = new Date(startedAt).getTime();
    if (!Number.isFinite(started)) return "";
    const elapsed = Math.max(0, Math.floor((clock - started) / 1000));
    return elapsed < 180
      ? `系统报告已生成 ${elapsed} 秒，通常需要 1–3 分钟，Windows 传输可能更久`
      : `系统报告已生成 ${elapsed} 秒，仍在等待设备生成和传输，超过约 5 分钟会自动结束`;
  };
  return (
    <div className="action-item post-capture-entry" id="field-postCapture">
      <button
        className="post-capture-trigger"
        disabled={disabled || starting || !serial}
        onClick={() => void show()}
        title="补采结果可在附件列表查看"
      >
        补采证据
        <span className="post-capture-info-badge" aria-hidden="true">
          i
        </span>
      </button>
      {running && <small role="status">正在补采，成功文件会加入附件</small>}
      {!open && error && (
        <small className="error-text" role="alert">
          {error}
        </small>
      )}
      {open && (
        <AppDialog
          title="补采证据"
          closeLabel="关闭补采"
          className="post-capture-dialog"
          onClose={() => setOpen(false)}
        >
          <p className="post-capture-context">
            问题类型：{draft.kind} · 目标应用：
            {draft.app.packageName || "未填写"}
            <br />
            目标设备：{serial || "未选择"}
          </p>
          <p className="post-capture-context">
            读取设备仍保留的证据。当前状态是补采时状态，不会修改原 Bug
            时间；录屏和复现附件请使用“导入附件”。
          </p>
          <div className="post-capture-options">
            {captureItems
              .filter((item) => item.id !== "environment")
              .map((item) => (
                <label className="check" key={item.id}>
                  <input
                    type="checkbox"
                    checked={selected.includes(item.id)}
                    disabled={starting || running || disabled}
                    onChange={(event) =>
                      setSelected((current) =>
                        event.target.checked
                          ? [...current, item.id]
                          : current.filter((id) => id !== item.id),
                      )
                    }
                  />
                  <span>
                    {item.name}
                    {completedItems.has(item.id) ? "（已采集）" : ""}
                  </span>
                </label>
              ))}
          </div>
          <p className="post-capture-context">
            已采集项默认不重复勾选；如需更新，重新勾选后会用本次结果替换该项旧附件。
          </p>
          <div className="actions">
            <button
              className="primary"
              disabled={
                disabled || starting || running || !serial || !selected.length
              }
              onClick={() => void begin(selected)}
            >
              {starting ? "准备采集…" : running ? "正在采集…" : "开始补采"}
            </button>
          </div>
          {error && (
            <small className="error-text" role="alert">
              {error}
            </small>
          )}
          {latest && latestResults.length > 0 && (
            <section className="post-capture-result" aria-label="补采结果">
              <div className="card-title">
                <strong>
                  {running ? "正在补采" : "最近补采结果（每项仅显示最新一次）"}
                </strong>
                <small>{new Date(latest.startedAt).toLocaleString()}</small>
              </div>
              <small>
                目标应用：{latest.targetPackage || "未填写"} · 设备：
                {latest.serial}
              </small>
              {latest.error && (
                <small className="error-text">{latest.error}</small>
              )}
              {latestResults.map(({ itemId, result, job }) => {
                const item = captureItems.find(
                  (candidate) => candidate.id === itemId,
                )!;
                const detail =
                  result.status === "running"
                    ? progressText(itemId, result.startedAt)
                    : "";
                return (
                  <div className="post-capture-result-item" key={itemId}>
                    <div>
                      <span>{item.name}</span>
                      <small>
                        {detail ||
                          captureStatusLabels[result.status] ||
                          result.status}
                      </small>
                    </div>
                    {result.error && (
                      <small className="error-text">{result.error}</small>
                    )}
                    {result.commands
                      ?.filter(
                        (command) =>
                          command.error && command.error !== result.error,
                      )
                      .map((command, index) => (
                        <small className="error-text" key={index}>
                          {command.error}
                        </small>
                      ))}
                    {result.commands
                      ?.filter((command) => command.warning)
                      .map((command, index) => (
                        <small
                          className="warning-text"
                          key={`warning-${index}`}
                        >
                          {command.warning}
                        </small>
                      ))}
                    {retryableItems(job).includes(itemId) && (
                      <button
                        disabled={disabled || running || starting || !serial}
                        onClick={() => void begin([itemId], job.id)}
                      >
                        重试{item.name}
                      </button>
                    )}
                  </div>
                );
              })}
            </section>
          )}
        </AppDialog>
      )}
    </div>
  );
}

export function EnvironmentSnapshotEntry({
  draft,
  serial,
  disabled,
  onPrepare,
  onUpdate,
  onComplete,
}: {
  draft: Draft;
  serial: string;
  disabled: boolean;
  onPrepare: () => Promise<unknown>;
  onUpdate: (snapshot: PostCaptureSnapshot) => void;
  onComplete: () => void;
}) {
  const [jobId, setJobId] = useState("");
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const update = useRef(onUpdate);
  const complete = useRef(onComplete);
  update.current = onUpdate;
  complete.current = onComplete;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const load = async () => {
    const snapshot = await invoke<PostCaptureSnapshot>("post_capture_status", {
      id: draft.id,
    });
    if (!alive.current) return;
    update.current(snapshot);
    const current = snapshot.jobs.find((job) => job.id === jobId);
    if (current && current.status !== "pending") {
      setJobId("");
      const item = current.items.environment;
      const failure = item?.error ?? current.error ?? "";
      if (current.status === "complete" && !failure) {
        complete.current();
      } else {
        setError(
          failure ||
            `设备环境快照${captureStatusLabels[current.status] ?? current.status}`,
        );
      }
    }
  };
  useEffect(() => {
    if (!jobId) return;
    let polling = false;
    const timer = window.setInterval(() => {
      if (polling) return;
      polling = true;
      void load()
        .catch((failure) => {
          if (alive.current)
            setError(`读取环境快照进度失败：${String(failure)}`);
        })
        .finally(() => {
          polling = false;
        });
    }, 800);
    return () => window.clearInterval(timer);
  }, [jobId, draft.id]);
  const begin = async () => {
    if (disabled || starting || jobId || !serial) return;
    setStarting(true);
    setError("");
    try {
      await onPrepare();
      const job = await invoke<PostCaptureJob>("start_post_capture", {
        id: draft.id,
        serial,
        items: ["environment"],
        retryOf: null,
      });
      if (!alive.current) return;
      setJobId(job.id);
      const snapshot = await invoke<PostCaptureSnapshot>(
        "post_capture_status",
        {
          id: draft.id,
        },
      );
      if (alive.current) update.current(snapshot);
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      if (alive.current) setStarting(false);
    }
  };
  return (
    <div className="action-item" id="field-environmentSnapshot">
      <button
        disabled={disabled || starting || Boolean(jobId) || !serial}
        onClick={() => void begin()}
      >
        设备环境快照
      </button>
      {(starting || jobId) && <small role="status">正在读取设备环境…</small>}
      {error && (
        <small className="error-text" role="alert">
          {error}
        </small>
      )}
    </div>
  );
}

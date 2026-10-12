import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Draft, Metadata, Profile, Submission } from "../shared/domain";
import type { HistoryPage } from "../shared/workspace";
import {
  emptyHistoryFilters,
  historyFields,
  historyOptions,
  localHistoryRows,
  type HistoryFilters,
} from "../shared/history";
import { CustomSelect } from "./custom-select";

interface Props {
  profile: Profile;
  metadata: Metadata | null;
  drafts: Draft[];
  submissions: Submission[];
  configured: boolean;
  busy: boolean;
  renderSubmission: (submission: Submission) => React.ReactNode;
  refresh: () => Promise<void>;
  remoteError: (error: unknown) => Promise<void>;
}
const pageSize = 20;
export function HistoryView({
  profile,
  metadata,
  drafts,
  submissions,
  configured,
  busy,
  renderSubmission,
  refresh,
  remoteError,
}: Props) {
  const [tab, setTab] = useState<"local" | "remote">("local");
  const [localFilters, setLocalFilters] =
    useState<HistoryFilters>(emptyHistoryFilters);
  const [remoteFilters, setRemoteFilters] =
    useState<HistoryFilters>(emptyHistoryFilters);
  const filters = tab === "local" ? localFilters : remoteFilters;
  const [applied, setApplied] = useState<HistoryFilters>(emptyHistoryFilters);
  const [localPage, setLocalPage] = useState(1);
  const [remote, setRemote] = useState<HistoryPage | null>(null);
  const [error, setError] = useState("");
  const [actionError, setActionError] = useState("");
  const [actionBusy, setActionBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const epoch = useRef(0);
  const alive = useRef(true);
  const requestBusy = useRef(false);
  const initialized = useRef(false);
  const remoteErrorRef = useRef(remoteError);
  remoteErrorRef.current = remoteError;
  const metadataValid =
    metadata?.profile.organizationId === profile.organizationId &&
    metadata?.profile.projectId === profile.projectId;
  const currentMetadata = metadataValid ? metadata : null;
  const scoped = submissions.filter(
    (s) =>
      s.profile.organizationId === profile.organizationId &&
      s.profile.projectId === profile.projectId,
  );
  const ids = new Set(scoped.map((s) => s.draftId));
  const localDrafts = drafts.filter((d) => ids.has(d.id));
  const local = configured
    ? localHistoryRows(submissions, drafts, profile, filters, currentMetadata)
    : [];
  const pending = JSON.stringify(filters) !== JSON.stringify(applied);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      epoch.current++;
    };
  }, []);
  useEffect(() => {
    setLocalPage((page) =>
      Math.min(page, Math.max(1, Math.ceil(local.length / pageSize))),
    );
  }, [local.length]);
  const load = async (nextFilters: HistoryFilters, more = false) => {
    if (!configured || requestBusy.current) return;
    requestBusy.current = true;
    const requestEpoch = ++epoch.current;
    setLoading(true);
    setError("");
    setActionError("");
    if (!more) {
      setRemote(null);
      setApplied(structuredClone(nextFilters));
    }
    try {
      const result = await invoke<HistoryPage>("list_remote_history", {
        page: more && remote ? remote.page + 1 : 1,
        filters: nextFilters,
        projectId: profile.projectId,
      });
      if (!alive.current || requestEpoch !== epoch.current) return;
      setRemote((old) =>
        more && old
          ? {
              ...result,
              items: [
                ...old.items,
                ...result.items.filter(
                  (item) => !old.items.some((v) => v.id === item.id),
                ),
              ],
            }
          : result,
      );
      initialized.current = true;
    } catch (failure) {
      if (!alive.current || requestEpoch !== epoch.current) return;
      setError(String(failure));
      await remoteErrorRef.current(failure);
    } finally {
      if (alive.current && requestEpoch === epoch.current) {
        setLoading(false);
        requestBusy.current = false;
      }
    }
  };
  useEffect(() => {
    if (tab === "remote" && !initialized.current && configured && !loading)
      void load(filters);
  }, [tab, configured]);
  const edit = (next: HistoryFilters) => {
    if (tab === "local") {
      setLocalFilters(next);
      setLocalPage(1);
      return;
    }
    setRemoteFilters(next);
    // A changed query must not show results or late responses from the old query.
    epoch.current++;
    requestBusy.current = false;
    setLoading(false);
    setRemote(null);
    setError("");
  };
  const switchTab = (next: "local" | "remote") => {
    if (next === tab) return;
    epoch.current++;
    requestBusy.current = false;
    if (loading) initialized.current = false;
    setLoading(false);
    setActionError("");
    setTab(next);
  };
  const runRemoteAction = async (
    command: "open_history_item" | "copy_history_link",
    identifier: string,
  ) => {
    if (actionBusy) return;
    setActionBusy(true);
    setActionError("");
    try {
      await invoke(command, { identifier });
    } catch (failure) {
      if (alive.current) setActionError(String(failure));
    } finally {
      if (alive.current) setActionBusy(false);
    }
  };
  return (
    <section className="card history-view">
      <div
        className="history-tabs"
        role="tablist"
        aria-label="历史记录来源"
        onKeyDown={(event) => {
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
            return;
          event.preventDefault();
          const next =
            event.key === "Home"
              ? "local"
              : event.key === "End"
                ? "remote"
                : tab === "local"
                  ? "remote"
                  : "local";
          switchTab(next);
          document.getElementById(`history-${next}-tab`)?.focus();
        }}
      >
        <button
          role="tab"
          tabIndex={tab === "local" ? 0 : -1}
          id="history-local-tab"
          aria-selected={tab === "local"}
          aria-controls="history-local-panel"
          onClick={() => switchTab("local")}
        >
          本机提交记录
        </button>
        <button
          role="tab"
          tabIndex={tab === "remote" ? 0 : -1}
          id="history-remote-tab"
          aria-selected={tab === "remote"}
          aria-controls="history-remote-panel"
          onClick={() => switchTab("remote")}
        >
          云效历史 Bug
        </button>
      </div>
      <div className="history-scroll">
        <div className="card-title history-heading">
          <h2>{tab === "local" ? "本机提交记录" : "云效历史 Bug"}</h2>
          <span className="pill">
            {configured ? profile.name : "请在主页面选择项目"}
          </span>
        </div>
        <div className="history-search">
          <label className="field">
            搜索历史
            <input
              disabled={loading}
              value={filters.keyword}
              placeholder={tab === "local" ? "标题或工作项编号" : "问题标题"}
              onChange={(e) => edit({ ...filters, keyword: e.target.value })}
            />
          </label>
        </div>
        <div className="grid history-filters">
          {historyFields.map(([key, label]) => (
            <label key={key}>
              {label}
              <CustomSelect
                label={`筛选${label}`}
                multiple
                autoSelectFirst={false}
                emptyLabel="不限"
                value={filters[key]}
                options={historyOptions(
                  key,
                  currentMetadata,
                  tab === "local" ? localDrafts : [],
                )}
                disabled={loading}
                onChange={(value) =>
                  edit({ ...filters, [key]: value as string[] })
                }
              />
            </label>
          ))}
        </div>
        <div className="actions history-filter-actions">
          <button
            className="primary"
            disabled={!configured || loading || busy}
            onClick={() =>
              tab === "remote" ? void load(filters) : setLocalPage(1)
            }
          >
            筛选
          </button>
          <button
            disabled={loading || busy}
            onClick={() => {
              const next = emptyHistoryFilters();
              edit(next);
              if (tab === "remote") void load(next);
            }}
          >
            清空条件
          </button>
          <button
            disabled={loading || busy || (tab === "remote" && !configured)}
            onClick={() =>
              tab === "remote" ? void load(filters) : void refresh()
            }
          >
            刷新
          </button>
          <span className="muted-text">
            {tab === "local"
              ? `共 ${local.length} 条 · 每页 ${pageSize} 条`
              : loading
                ? "正在读取…"
                : pending
                  ? "条件已变更，点击筛选"
                  : `已加载 ${remote?.items.length ?? 0} 条`}
          </span>
        </div>
        <div
          className="history-results"
          role="tabpanel"
          id={tab === "local" ? "history-local-panel" : "history-remote-panel"}
          aria-labelledby={
            tab === "local" ? "history-local-tab" : "history-remote-tab"
          }
          aria-busy={loading}
        >
          {tab === "local" ? (
            <>
              {local.length ? (
                local
                  .slice((localPage - 1) * pageSize, localPage * pageSize)
                  .map(renderSubmission)
              ) : (
                <div className="empty">
                  <p>当前项目下没有符合筛选条件的本机提交记录。</p>
                </div>
              )}
              {local.length > pageSize && (
                <div className="actions history-pagination">
                  <button
                    disabled={localPage === 1}
                    onClick={() => setLocalPage((v) => v - 1)}
                  >
                    上一页
                  </button>
                  <span>
                    第 {localPage} / {Math.ceil(local.length / pageSize)} 页
                  </span>
                  <button
                    disabled={localPage * pageSize >= local.length}
                    onClick={() => setLocalPage((v) => v + 1)}
                  >
                    下一页
                  </button>
                </div>
              )}
            </>
          ) : (
            <>
              {error && (
                <p className="banner error" role="alert">
                  云效历史读取失败：{error}
                </p>
              )}
              {actionError && (
                <p className="banner error" role="alert">
                  云效历史操作失败：{actionError}
                </p>
              )}
              {remote &&
                (remote.items.length ? (
                  remote.items.map((item) => (
                    <article className="submission" key={item.id}>
                      <div className="card-title">
                        <h3>{item.title}</h3>
                        <span className="pill">{item.status}</span>
                      </div>
                      <p>
                        工作项 {item.identifier || item.id} · 创建于{" "}
                        {item.createdAt}
                      </p>
                      {item.identifier && (
                        <div className="actions">
                          <button
                            disabled={actionBusy || busy}
                            onClick={() =>
                              void runRemoteAction(
                                "open_history_item",
                                item.identifier,
                              )
                            }
                          >
                            打开云效
                          </button>
                          <button
                            disabled={actionBusy || busy}
                            onClick={() =>
                              void runRemoteAction(
                                "copy_history_link",
                                item.identifier,
                              )
                            }
                          >
                            复制链接
                          </button>
                        </div>
                      )}
                    </article>
                  ))
                ) : (
                  <div className="empty">
                    <p>
                      {remote.hasMore
                        ? "本批次没有匹配记录，可继续加载后续记录。"
                        : "当前账号在此项目下没有符合筛选条件的历史 Bug。"}
                    </p>
                  </div>
                ))}
              {!remote && !loading && !error && (
                <div className="empty">
                  <p>
                    {configured
                      ? "点击筛选读取历史 Bug。"
                      : "请先在主页面选择项目。"}
                  </p>
                </div>
              )}
              {remote?.hasMore && (
                <button
                  disabled={loading || busy || pending}
                  onClick={() => void load(applied, true)}
                >
                  加载更多
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </section>
  );
}

import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Profile } from "../shared/domain";
import {
  emptyProjectBugFilters,
  projectBugDateError,
  projectBugQuery,
  projectBugSortOptions,
  type ProjectBugFacets,
  type ProjectBugFilters,
  type ProjectBugPage,
} from "../shared/project-bugs";
import { CustomSelect } from "./custom-select";

interface Props {
  profile: Profile;
  configured: boolean;
  busy: boolean;
  remoteError: (error: unknown) => Promise<void>;
}

const emptyFacets: ProjectBugFacets = {
  statuses: [],
  members: [],
  tags: [],
  warnings: [],
};

function displayTime(value: string) {
  if (!value) return "—";
  const parsed = new Date(value.replace(" ", "T"));
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleString("zh-CN", { hour12: false });
}

export function ProjectBugsView({
  profile,
  configured,
  busy,
  remoteError,
}: Props) {
  const [filters, setFilters] = useState<ProjectBugFilters>(
    emptyProjectBugFilters,
  );
  const [applied, setApplied] = useState<ProjectBugFilters>(
    emptyProjectBugFilters,
  );
  const [facets, setFacets] = useState<ProjectBugFacets>(emptyFacets);
  const [page, setPage] = useState<ProjectBugPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [facetsLoading, setFacetsLoading] = useState(false);
  const [error, setError] = useState("");
  const [facetsError, setFacetsError] = useState("");
  const [actionError, setActionError] = useState("");
  const [actionBusy, setActionBusy] = useState(false);
  const alive = useRef(true);
  const listEpoch = useRef(0);
  const facetEpoch = useRef(0);
  const remoteErrorRef = useRef(remoteError);
  remoteErrorRef.current = remoteError;
  const pending = JSON.stringify(filters) !== JSON.stringify(applied);

  const loadFacets = async () => {
    if (!configured) return;
    const epoch = ++facetEpoch.current;
    setFacetsLoading(true);
    setFacetsError("");
    try {
      const result = await invoke<ProjectBugFacets>("project_bug_facets", {
        projectId: profile.projectId,
      });
      if (!alive.current || epoch !== facetEpoch.current) return;
      setFacets(result);
    } catch (failure) {
      if (!alive.current || epoch !== facetEpoch.current) return;
      setFacetsError(String(failure));
      await remoteErrorRef.current(failure);
    } finally {
      if (alive.current && epoch === facetEpoch.current)
        setFacetsLoading(false);
    }
  };

  const load = async (next: ProjectBugFilters, targetPage = 1) => {
    if (!configured) return;
    const dateError = projectBugDateError(next);
    if (dateError) {
      setError(dateError);
      return;
    }
    const epoch = ++listEpoch.current;
    setLoading(true);
    setError("");
    setActionError("");
    setApplied(structuredClone(next));
    try {
      const result = await invoke<ProjectBugPage>("list_project_bugs", {
        query: projectBugQuery(next, targetPage),
        projectId: profile.projectId,
      });
      if (!alive.current || epoch !== listEpoch.current) return;
      setPage(result);
    } catch (failure) {
      if (!alive.current || epoch !== listEpoch.current) return;
      setPage(null);
      setError(String(failure));
      await remoteErrorRef.current(failure);
    } finally {
      if (alive.current && epoch === listEpoch.current) setLoading(false);
    }
  };

  useEffect(() => {
    alive.current = true;
    if (configured) {
      const initial = emptyProjectBugFilters();
      void loadFacets();
      void load(initial);
    }
    return () => {
      alive.current = false;
      listEpoch.current += 1;
      facetEpoch.current += 1;
    };
  }, []);

  const edit = (next: ProjectBugFilters) => {
    listEpoch.current += 1;
    setLoading(false);
    setFilters(next);
    setError("");
  };

  const open = async (identifier: string) => {
    if (!identifier || actionBusy) return;
    setActionBusy(true);
    setActionError("");
    try {
      await invoke("open_history_item", { identifier });
    } catch (failure) {
      if (alive.current) setActionError(String(failure));
    } finally {
      if (alive.current) setActionBusy(false);
    }
  };

  return (
    <section className="card project-bugs-view">
      <div className="project-bugs-scroll">
        <div className="card-title project-bugs-heading">
          <div>
            <h2>项目 Bug</h2>
            <p className="muted-text">当前项目的全部云效 Bug</p>
          </div>
          <span className="pill">
            {configured ? profile.name : "请在主页面选择项目"}
          </span>
        </div>

        <div className="project-bug-search">
          <label className="field">
            标题
            <input
              value={filters.keyword}
              disabled={loading}
              placeholder="输入标题关键词"
              onChange={(event) =>
                edit({ ...filters, keyword: event.target.value })
              }
              onKeyDown={(event) => {
                if (event.key === "Enter") void load(filters);
              }}
            />
          </label>
        </div>

        <div className="grid project-bug-filters">
          <label>
            状态
            <CustomSelect
              multiple
              autoSelectFirst={false}
              emptyLabel="不限"
              label="筛选状态"
              value={filters.statusIds}
              options={facets.statuses}
              disabled={loading || facetsLoading}
              onChange={(value) =>
                edit({ ...filters, statusIds: value as string[] })
              }
            />
          </label>
          <label>
            负责人
            <CustomSelect
              multiple
              autoSelectFirst={false}
              emptyLabel="不限"
              label="筛选负责人"
              value={filters.assignedToIds}
              options={facets.members}
              disabled={loading || facetsLoading}
              onChange={(value) =>
                edit({ ...filters, assignedToIds: value as string[] })
              }
            />
          </label>
          <label>
            创建者
            <CustomSelect
              multiple
              autoSelectFirst={false}
              emptyLabel="不限"
              label="筛选创建者"
              value={filters.creatorIds}
              options={facets.members}
              disabled={loading || facetsLoading}
              onChange={(value) =>
                edit({ ...filters, creatorIds: value as string[] })
              }
            />
          </label>
          <label>
            标签
            <CustomSelect
              multiple
              autoSelectFirst={false}
              emptyLabel="不限"
              label="筛选标签"
              value={filters.tagIds}
              options={facets.tags}
              disabled={loading || facetsLoading}
              onChange={(value) =>
                edit({ ...filters, tagIds: value as string[] })
              }
            />
          </label>
          <label>
            创建日期从
            <input
              type="date"
              value={filters.createdFrom}
              disabled={loading}
              onChange={(event) =>
                edit({ ...filters, createdFrom: event.target.value })
              }
            />
          </label>
          <label>
            创建日期到
            <input
              type="date"
              value={filters.createdTo}
              disabled={loading}
              onChange={(event) =>
                edit({ ...filters, createdTo: event.target.value })
              }
            />
          </label>
          <label>
            排序
            <CustomSelect
              value={filters.sort}
              options={projectBugSortOptions}
              label="排序方式"
              disabled={loading}
              onChange={(value) =>
                edit({
                  ...filters,
                  sort: value as ProjectBugFilters["sort"],
                })
              }
            />
          </label>
        </div>

        <div className="actions project-bug-filter-actions">
          <button
            className="primary"
            disabled={!configured || loading || busy}
            onClick={() => void load(filters)}
          >
            筛选
          </button>
          <button
            disabled={loading || busy}
            onClick={() => {
              const next = emptyProjectBugFilters();
              setFilters(next);
              void load(next);
            }}
          >
            清空条件
          </button>
          <button
            disabled={!configured || loading || busy}
            onClick={() => {
              void loadFacets();
              void load(applied, page?.page ?? 1);
            }}
          >
            刷新
          </button>
          <span className="muted-text">
            {loading
              ? "正在读取…"
              : pending
                ? "条件已变更，点击筛选"
                : page?.total !== null && page?.total !== undefined
                  ? `共 ${page.total} 条`
                  : `本页 ${page?.items.length ?? 0} 条`}
          </span>
        </div>

        {facets.warnings.map((warning) => (
          <p className="banner warning" key={warning}>
            {warning}
          </p>
        ))}
        {facetsError && (
          <p className="banner warning" role="alert">
            筛选项读取失败：{facetsError}
          </p>
        )}
        {error && (
          <p className="banner error" role="alert">
            项目 Bug 读取失败：{error}
          </p>
        )}
        {actionError && (
          <p className="banner error" role="alert">
            打开云效失败：{actionError}
          </p>
        )}

        <div className="project-bug-table-wrap" aria-busy={loading}>
          <table className="project-bug-table">
            <thead>
              <tr>
                <th>标题</th>
                <th>状态</th>
                <th>负责人</th>
                <th>创建者</th>
                <th>创建时间</th>
                <th>更新时间</th>
                <th aria-label="操作" />
              </tr>
            </thead>
            <tbody>
              {page?.items.map((item) => (
                <tr key={item.id}>
                  <td>
                    <strong>{item.title || "未命名 Bug"}</strong>
                    <small>
                      {item.identifier || item.id}
                      {item.labels.length
                        ? ` · ${item.labels.map((label) => label.name).join("、")}`
                        : ""}
                    </small>
                  </td>
                  <td>
                    <span className="pill">
                      {item.status?.name || "未提供"}
                    </span>
                  </td>
                  <td>{item.assignedTo?.name || "未分配"}</td>
                  <td>{item.creator?.name || "—"}</td>
                  <td>{displayTime(item.createdAt)}</td>
                  <td>{displayTime(item.updatedAt)}</td>
                  <td>
                    <button
                      disabled={!item.identifier || actionBusy || busy}
                      onClick={() => void open(item.identifier)}
                    >
                      打开
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {!loading && page && !page.items.length && (
            <div className="empty">
              <p>当前项目下没有符合筛选条件的 Bug。</p>
            </div>
          )}
          {!loading && !page && !error && (
            <div className="empty">
              <p>选择项目后读取云效 Bug。</p>
            </div>
          )}
        </div>

        {page && (page.page > 1 || page.hasMore) && (
          <div className="actions project-bug-pagination">
            <button
              disabled={loading || pending || page.page <= 1}
              onClick={() => void load(applied, page.page - 1)}
            >
              上一页
            </button>
            <span>
              第 {page.page}
              {page.totalPages ? ` / ${page.totalPages}` : ""} 页
            </span>
            <button
              disabled={loading || pending || !page.hasMore}
              onClick={() => void load(applied, page.page + 1)}
            >
              下一页
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

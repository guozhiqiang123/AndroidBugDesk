import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";
import type { Option, Profile } from "../shared/domain";
import type {
  ProjectBugActivity,
  ProjectBugAttachment,
  ProjectBugAttachmentPreview,
  ProjectBugContentFormat,
  ProjectBugDetail,
  ProjectBugItem,
} from "../shared/project-bugs";
import { AppDialog } from "./app-dialog";
import { MarkdownPreview } from "./markdown-editor";
import "./project-bug-detail-dialog.css";

interface Props {
  item: ProjectBugItem;
  detail: ProjectBugDetail | null;
  profile: Profile;
  loading: boolean;
  error: string;
  openError: string;
  openingWeb: boolean;
  onClose: () => void;
  onRefresh: () => void;
  onOpenWeb: () => void;
  remoteError: (error: unknown) => Promise<void>;
}

function displayTime(value: string) {
  if (!value) return "—";
  const parsed = new Date(value.replace(" ", "T"));
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleString("zh-CN", { hour12: false });
}

function displaySize(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "大小未知";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(2)} MB`;
}

function names(values: Option[]) {
  return values.length ? values.map((value) => value.name).join("、") : "—";
}

function contentFormatLabel(format: ProjectBugContentFormat) {
  if (format === "markdown") return "Markdown";
  if (format === "richtext") return "富文本";
  return "纯文本";
}

function safeRichText(value: string) {
  const clean = DOMPurify.sanitize(value, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
      "style",
      "form",
      "iframe",
      "object",
      "embed",
      "svg",
      "math",
      "video",
      "audio",
    ],
    FORBID_ATTR: ["style", "srcset", "formaction", "target"],
  });
  const documentValue = new DOMParser().parseFromString(clean, "text/html");
  for (const link of documentValue.querySelectorAll("a")) {
    const href = link.getAttribute("href") ?? "";
    link.removeAttribute("href");
    if (href) link.setAttribute("title", "外部链接请在云效网页中打开");
  }
  for (const image of documentValue.querySelectorAll("img")) {
    const source = image.getAttribute("src") ?? "";
    image.removeAttribute("src");
    image.removeAttribute("srcset");
    if (source) image.dataset.remoteSource = source;
    image.setAttribute("loading", "lazy");
  }
  return documentValue.body.innerHTML;
}

function attachmentCandidates(source: string, alt: string) {
  const candidates = new Set<string>();
  if (alt.trim()) candidates.add(alt.trim());
  try {
    const parsed = new URL(source);
    const name = decodeURIComponent(parsed.pathname.split("/").at(-1) ?? "");
    if (name) candidates.add(name);
  } catch {
    const name = source.split(/[?#]/)[0].split("/").at(-1) ?? "";
    if (name) candidates.add(name);
  }
  return candidates;
}

function matchingImage(
  attachments: ProjectBugAttachment[],
  source: string,
  alt: string,
) {
  const candidates = attachmentCandidates(source, alt);
  return attachments.find((attachment) => {
    const suffix = attachment.suffix.toLowerCase();
    if (![".png", ".jpg", ".jpeg", "png", "jpg", "jpeg"].includes(suffix))
      return false;
    return candidates.has(attachment.name);
  });
}

function ContentPreview({
  value,
  format,
  attachments,
  readImage,
}: {
  value: string;
  format: ProjectBugContentFormat;
  attachments: ProjectBugAttachment[];
  readImage: (attachment: ProjectBugAttachment) => Promise<string>;
}) {
  const richRef = useRef<HTMLDivElement>(null);
  const html = useMemo(
    () => (format === "richtext" ? safeRichText(value) : ""),
    [format, value],
  );

  useEffect(() => {
    if (format !== "richtext" || !richRef.current) return;
    let active = true;
    for (const image of richRef.current.querySelectorAll("img")) {
      const source = image.dataset.remoteSource ?? "";
      const attachment = matchingImage(
        attachments,
        source,
        image.getAttribute("alt") ?? "",
      );
      if (!attachment) {
        image.replaceWith(
          Object.assign(document.createElement("span"), {
            className: "project-bug-inline-image-missing",
            textContent: "正文图片可在附件区预览",
          }),
        );
        continue;
      }
      void readImage(attachment)
        .then((source) => {
          if (active && image.isConnected) image.src = source;
        })
        .catch(() => {
          if (active && image.isConnected) {
            image.replaceWith(
              Object.assign(document.createElement("span"), {
                className: "project-bug-inline-image-missing",
                textContent: `图片 ${attachment.name} 读取失败，可在附件区重试`,
              }),
            );
          }
        });
    }
    return () => {
      active = false;
    };
  }, [attachments, format, html, readImage]);

  if (!value.trim()) return <p className="empty-text">未填写描述</p>;
  if (format === "markdown") {
    return (
      <div
        className="project-bug-content"
        onClickCapture={(event) => {
          if ((event.target as HTMLElement).closest("a"))
            event.preventDefault();
        }}
      >
        <MarkdownPreview
          value={value}
          onReadAsset={() => Promise.reject(new Error("非本地正文资源"))}
          onReadRemoteImage={(source, alt) => {
            const attachment = matchingImage(attachments, source, alt);
            return attachment
              ? readImage(attachment)
              : Promise.reject(new Error("正文图片未匹配附件"));
          }}
        />
      </div>
    );
  }
  if (format === "richtext") {
    return (
      <div
        ref={richRef}
        className="project-bug-content project-bug-richtext"
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  }
  return <pre className="project-bug-plain-content">{value}</pre>;
}

function activityText(activity: ProjectBugActivity) {
  const eventNames: Record<string, string> = {
    "workitem.created": "创建了工作项",
    "workitem.updated": "更新了工作项",
    "workitem.transitioned": "流转了状态",
    "workitem.association.changed": "调整了关联项",
    "workitem.attachment.changed": "调整了附件",
  };
  return (
    eventNames[activity.eventType] ||
    (activity.propertyName
      ? `更新了${activity.propertyName}`
      : activity.actionType || "更新了工作项")
  );
}

export function ProjectBugDetailDialog({
  item,
  detail,
  profile,
  loading,
  error,
  openError,
  openingWeb,
  onClose,
  onRefresh,
  onOpenWeb,
  remoteError,
}: Props) {
  const [attachmentBusy, setAttachmentBusy] = useState("");
  const [attachmentError, setAttachmentError] = useState("");
  const [attachmentNotice, setAttachmentNotice] = useState("");
  const [preview, setPreview] = useState<ProjectBugAttachmentPreview | null>(
    null,
  );
  const previewCache = useRef(new Map<string, ProjectBugAttachmentPreview>());

  const readAttachment = useCallback(
    async (attachment: ProjectBugAttachment) => {
      const cached = previewCache.current.get(attachment.id);
      if (cached) return cached;
      const value = await invoke<ProjectBugAttachmentPreview>(
        "preview_project_bug_attachment",
        {
          id: item.id,
          attachmentId: attachment.id,
          projectId: profile.projectId,
        },
      );
      if (value.kind !== "system")
        previewCache.current.set(attachment.id, value);
      return value;
    },
    [item.id, profile.projectId],
  );

  const readImage = useCallback(
    async (attachment: ProjectBugAttachment) => {
      const result = await readAttachment(attachment);
      if (result.kind !== "image") throw new Error("附件不是可预览图片");
      return result.data;
    },
    [readAttachment],
  );

  const openAttachment = async (attachment: ProjectBugAttachment) => {
    if (attachmentBusy) return;
    setAttachmentBusy(attachment.id);
    setAttachmentError("");
    setAttachmentNotice("");
    try {
      const value = await readAttachment(attachment);
      if (value.kind === "system") {
        setPreview(null);
        setAttachmentNotice(`已使用系统应用打开 ${value.name}`);
      } else {
        setPreview(value);
      }
    } catch (failure) {
      setAttachmentError(String(failure));
      await remoteError(failure);
    } finally {
      setAttachmentBusy("");
    }
  };

  const title = `${item.identifier || "Bug"} · 详情`;
  return (
    <AppDialog
      title={title}
      onClose={onClose}
      className="project-bug-detail-dialog"
    >
      <div className="project-bug-detail-toolbar">
        <div>
          <h3>{detail?.title || item.title || "未命名 Bug"}</h3>
          <div className="project-bug-detail-badges">
            <span className="pill">
              {detail?.status?.name || item.status?.name || "未提供状态"}
            </span>
            {detail?.archived && <span className="pill muted">已归档</span>}
          </div>
        </div>
        <div className="actions">
          <button disabled={loading} onClick={onRefresh}>
            {loading ? "正在刷新…" : "刷新"}
          </button>
          <button className="primary" disabled={openingWeb} onClick={onOpenWeb}>
            {openingWeb ? "正在打开…" : "在云效中打开"}
          </button>
        </div>
      </div>

      {error && (
        <div className="banner error" role="alert">
          <span>详情读取失败：{error}</span>
          <button onClick={onRefresh}>重试</button>
        </div>
      )}
      {openError && (
        <div className="banner error" role="alert">
          打开云效失败：{openError}
        </div>
      )}
      {loading && !detail && (
        <div className="project-bug-detail-loading">正在读取完整详情…</div>
      )}

      {detail && (
        <div className="project-bug-detail-sections" aria-busy={loading}>
          <section className="project-bug-detail-section">
            <div className="project-bug-detail-summary">
              <div>
                <span>负责人</span>
                <strong>{detail.assignedTo?.name || "未分配"}</strong>
              </div>
              <div>
                <span>创建者</span>
                <strong>{detail.creator?.name || "—"}</strong>
              </div>
              <div>
                <span>验证者</span>
                <strong>{detail.verifier?.name || "—"}</strong>
              </div>
              <div>
                <span>迭代</span>
                <strong>{detail.sprint?.name || "—"}</strong>
              </div>
              <div>
                <span>创建时间</span>
                <strong>{displayTime(detail.createdAt)}</strong>
              </div>
              <div>
                <span>更新时间</span>
                <strong>{displayTime(detail.updatedAt)}</strong>
              </div>
              <div>
                <span>参与者</span>
                <strong>{names(detail.participants)}</strong>
              </div>
              <div>
                <span>抄送</span>
                <strong>{names(detail.trackers)}</strong>
              </div>
              <div className="wide">
                <span>标签</span>
                <strong>{names(detail.labels)}</strong>
              </div>
            </div>
          </section>

          <section className="project-bug-detail-section">
            <div className="project-bug-section-title">
              <h4>问题描述</h4>
              <span>{contentFormatLabel(detail.format)}</span>
            </div>
            <ContentPreview
              value={detail.description}
              format={detail.format}
              attachments={detail.attachments}
              readImage={readImage}
            />
          </section>

          <section className="project-bug-detail-section">
            <div className="project-bug-section-title">
              <h4>字段信息</h4>
              <span>{detail.customFields.length} 项</span>
            </div>
            {detail.customFields.length ? (
              <dl className="project-bug-custom-fields">
                {detail.customFields.map((field, index) => (
                  <div key={`${field.id || field.name}:${index}`}>
                    <dt>{field.name || field.id}</dt>
                    <dd>
                      {field.values.length ? field.values.join("、") : "未填写"}
                    </dd>
                  </div>
                ))}
              </dl>
            ) : (
              <p className="empty-text">没有额外字段</p>
            )}
          </section>

          <section className="project-bug-detail-section">
            <div className="project-bug-section-title">
              <h4>附件</h4>
              <span>{detail.attachments.length} 个</span>
            </div>
            {detail.sectionWarnings
              .filter((value) => value.section === "attachments")
              .map((warning) => (
                <p className="banner warning" key={warning.message}>
                  {warning.message}
                </p>
              ))}
            {detail.attachments.length ? (
              <div className="project-bug-attachments">
                {detail.attachments.map((attachment) => (
                  <div
                    key={attachment.id}
                    className="project-bug-attachment-row"
                  >
                    <div>
                      <strong>{attachment.name || "未命名附件"}</strong>
                      <small>
                        {displaySize(attachment.size)} ·{" "}
                        {attachment.creator?.name || "未知上传人"} ·{" "}
                        {displayTime(attachment.createdAt)}
                      </small>
                    </div>
                    <button
                      disabled={!!attachmentBusy}
                      onClick={() => void openAttachment(attachment)}
                    >
                      {attachmentBusy === attachment.id ? "正在读取…" : "预览"}
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <p className="empty-text">没有附件</p>
            )}
            {attachmentError && (
              <p className="banner error" role="alert">
                附件预览失败：{attachmentError}
              </p>
            )}
            {attachmentNotice && (
              <p className="banner success">{attachmentNotice}</p>
            )}
            {preview && preview.kind !== "system" && (
              <div className="project-bug-attachment-preview">
                <div className="project-bug-section-title">
                  <h4>{preview.name}</h4>
                  <button
                    onClick={() => setPreview(null)}
                    aria-label="关闭附件预览"
                  >
                    关闭
                  </button>
                </div>
                {preview.kind === "image" ? (
                  <img src={preview.data} alt={preview.name} />
                ) : (
                  <pre>{preview.data}</pre>
                )}
              </div>
            )}
          </section>

          <section className="project-bug-detail-section">
            <div className="project-bug-section-title">
              <h4>评论</h4>
              <span>{detail.comments.length} 条</span>
            </div>
            {detail.sectionWarnings
              .filter((value) => value.section === "comments")
              .map((warning) => (
                <p className="banner warning" key={warning.message}>
                  {warning.message}
                </p>
              ))}
            {detail.comments.length ? (
              detail.comments.map((comment) => (
                <article
                  className={`project-bug-comment${comment.parentId ? " reply" : ""}`}
                  key={comment.id}
                >
                  <header>
                    <strong>{comment.author?.name || "未知用户"}</strong>
                    <span>
                      {comment.pinned ? "置顶 · " : ""}
                      {comment.parentId ? "回复 · " : ""}
                      {displayTime(comment.createdAt)}
                    </span>
                  </header>
                  <ContentPreview
                    value={comment.content}
                    format={comment.format}
                    attachments={detail.attachments}
                    readImage={readImage}
                  />
                </article>
              ))
            ) : (
              <p className="empty-text">暂无评论</p>
            )}
          </section>

          <section className="project-bug-detail-section">
            <div className="project-bug-section-title">
              <h4>操作动态</h4>
              <span>{detail.activities.length} 条</span>
            </div>
            {detail.sectionWarnings
              .filter((value) => value.section === "activities")
              .map((warning) => (
                <p className="banner warning" key={warning.message}>
                  {warning.message}
                </p>
              ))}
            {detail.activities.length ? (
              <ol className="project-bug-activities">
                {detail.activities.map((activity, index) => (
                  <li key={`${activity.id}:${index}`}>
                    <span
                      className="project-bug-activity-dot"
                      aria-hidden="true"
                    />
                    <div>
                      <strong>
                        {activity.actor?.name || "系统"}{" "}
                        {activityText(activity)}
                      </strong>
                      <small>{displayTime(activity.occurredAt)}</small>
                      {(activity.oldValues.length > 0 ||
                        activity.newValues.length > 0) && (
                        <p>
                          <span>{activity.oldValues.join("、") || "空"}</span>
                          <b>→</b>
                          <span>{activity.newValues.join("、") || "空"}</span>
                        </p>
                      )}
                    </div>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="empty-text">暂无操作动态</p>
            )}
          </section>
        </div>
      )}
    </AppDialog>
  );
}

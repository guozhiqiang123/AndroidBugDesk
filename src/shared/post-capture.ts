import type { Draft } from "./domain";
export const captureItems = [
  { id: "environment", name: "设备与应用环境" },
  { id: "logs", name: "现存日志" },
  { id: "screenshot", name: "当前截图" },
  { id: "ui", name: "当前界面层级" },
  { id: "app", name: "应用与进程状态" },
  { id: "activities", name: "Activity任务栈" },
  { id: "display", name: "窗口与显示状态" },
  { id: "locale", name: "语言与字体配置" },
  { id: "crash", name: "崩溃日志" },
  { id: "exit", name: "历史进程退出记录" },
  { id: "memory", name: "当前应用内存" },
  { id: "frames", name: "现存帧统计" },
  { id: "cpu", name: "CPU与进程状态" },
  { id: "network", name: "网络配置与统计" },
  { id: "storage", name: "存储空间与权限" },
  { id: "bugreport", name: "系统报告（整机级，按需）" },
] as const;
export type CaptureItemId = (typeof captureItems)[number]["id"];
const captureItemName = (id: CaptureItemId) =>
  captureItems.find((item) => item.id === id)!.name;

export function describePostCaptureAttachment(name: string, source: string) {
  if (source.includes("采集项：")) return source;
  let label = "";
  if (name === "post-capture-summary.zip") {
    label = "本次补采汇总";
  } else if (name === "post-capture-current-screen.png") {
    label = captureItemName("screenshot");
  } else if (name.startsWith("device-environment-snapshot-")) {
    label = captureItemName("environment");
  } else {
    const match = /^post-capture-([a-z]+)\.zip$/u.exec(name);
    const item = match?.[1] as CaptureItemId | undefined;
    if (item && captureItems.some((candidate) => candidate.id === item)) {
      label = captureItemName(item);
    } else if (source.startsWith("事后补采系统报告")) {
      label = captureItemName("bugreport");
    }
  }
  return label ? `采集项：${label}；${source}` : source;
}

export function splitPostCaptureAttachmentDescription(
  name: string,
  source: string,
) {
  const description = describePostCaptureAttachment(name, source);
  const parts = description.split("；");
  const captureItemIndex = parts.findIndex((part) =>
    part.startsWith("采集项："),
  );
  if (captureItemIndex < 0) {
    return { captureItem: "", description };
  }
  const captureItem = parts[captureItemIndex].slice("采集项：".length).trim();
  parts.splice(captureItemIndex, 1);
  return {
    captureItem,
    description: parts.filter(Boolean).join("；"),
  };
}

const extras: Record<Draft["kind"], CaptureItemId[]> = {
  功能: ["ui", "app", "activities"],
  UI: ["ui", "display", "activities"],
  多语言: ["ui", "locale", "display"],
  动态过程: ["ui", "app", "activities"],
  崩溃: ["crash", "exit", "app", "activities"],
  ANR: ["crash", "exit", "app", "activities", "display", "memory", "bugreport"],
  广告: ["app", "activities"],
  性能: ["memory", "frames", "cpu"],
  网络: ["network"],
  数据: ["storage", "app"],
};
export function recommendedCaptureItems(kind: Draft["kind"]): CaptureItemId[] {
  // Whole-device bugreport is absent from most default sets. ANR keeps it
  // because thread and system blocking context cannot be reconstructed
  // reliably from the narrower after-the-fact commands.
  return ["logs", "screenshot", ...extras[kind]];
}
export interface CaptureItemResult {
  status: string;
  error?: string;
  attachments: string[];
  startedAt?: string;
  finishedAt?: string;
  commands?: {
    status?: string;
    error?: string;
    warning?: string;
    source?: string;
    matchedRecordCount?: number;
    discardedRecordCount?: number;
    rawDeviceLogPersisted?: boolean;
    skipped?: boolean;
    argv: string[];
  }[];
}
export interface PostCaptureJob {
  id: string;
  draftId: string;
  kind: "post-capture";
  status: string;
  serial: string;
  targetPackage: string;
  androidApiLevel?: number | null;
  issueKind: Draft["kind"];
  startedAt: string;
  finishedAt?: string;
  error?: string;
  items: Partial<Record<CaptureItemId, CaptureItemResult>>;
}
export interface PostCaptureSnapshot {
  draft: Draft;
  jobs: PostCaptureJob[];
}
export interface LatestCaptureResult {
  itemId: CaptureItemId;
  result: CaptureItemResult;
  job: PostCaptureJob;
}
export const captureStatusLabels: Record<string, string> = {
  pending: "等待采集",
  running: "采集中",
  complete: "已获取",
  partial: "部分获取",
  unavailable: "未获取到",
  failed: "采集失败",
  interrupted: "采集中断",
};
export function retryableItems(job: PostCaptureJob): CaptureItemId[] {
  return captureItems
    .filter(({ id }) => {
      const status = job.items[id]?.status;
      return (
        status &&
        (["failed", "unavailable", "partial", "interrupted"].includes(status) ||
          (job.status === "interrupted" &&
            ["pending", "running"].includes(status)))
      );
    })
    .map(({ id }) => id);
}

export function latestCaptureResults(
  jobs: PostCaptureJob[],
): LatestCaptureResult[] {
  const latest = new Map<CaptureItemId, LatestCaptureResult>();
  for (const job of jobs) {
    for (const item of captureItems) {
      const result = job.items[item.id];
      if (result && !latest.has(item.id))
        latest.set(item.id, { itemId: item.id, result, job });
    }
  }
  return captureItems.flatMap((item) => {
    const result = latest.get(item.id);
    return result ? [result] : [];
  });
}

export function recommendedUncollectedItems(
  kind: Draft["kind"],
  jobs: PostCaptureJob[],
): CaptureItemId[] {
  const latest = new Map(
    latestCaptureResults(jobs).map((entry) => [entry.itemId, entry.result]),
  );
  return recommendedCaptureItems(kind).filter(
    (item) => latest.get(item)?.status !== "complete",
  );
}

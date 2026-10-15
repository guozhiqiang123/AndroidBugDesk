import { describe, expect, it } from "vitest";
import {
  captureItems,
  describePostCaptureAttachment,
  latestCaptureResults,
  recommendedCaptureItems,
  recommendedUncollectedItems,
  retryableItems,
  splitPostCaptureAttachmentDescription,
  type PostCaptureJob,
} from "../src/shared/post-capture";
describe("事后补采组合和重试", () => {
  it("新旧补采附件说明都显示对应采集项", () => {
    expect(
      describePostCaptureAttachment(
        "post-capture-logs.zip",
        "事后补采诊断；时间及范围见 receipt.json",
      ),
    ).toBe("采集项：现存日志；事后补采诊断；时间及范围见 receipt.json");
    expect(
      describePostCaptureAttachment(
        "post-capture-current-screen.png",
        "事后补采当前截图；采集于 2026-10-10T10:46:19Z",
      ),
    ).toContain("采集项：当前截图");
    expect(
      describePostCaptureAttachment(
        "bugreport.zip",
        "事后补采系统报告；采集于 2026-10-10T10:46:19Z",
      ),
    ).toContain("采集项：系统报告");
    expect(
      describePostCaptureAttachment(
        "post-capture-ui.zip",
        "事后补采诊断；采集项：当前界面层级；时间及范围见 receipt.json",
      ),
    ).toBe("事后补采诊断；采集项：当前界面层级；时间及范围见 receipt.json");
    expect(
      describePostCaptureAttachment(
        "post-capture-activities.zip",
        "事后补采诊断；时间及范围见 receipt.json",
      ),
    ).toContain("采集项：Activity任务栈");
    expect(describePostCaptureAttachment("manual.png", "用户导入文件")).toBe(
      "用户导入文件",
    );
  });

  it("补采附件说明可拆成独立采集项和来源描述", () => {
    expect(
      splitPostCaptureAttachmentDescription(
        "post-capture-logs.zip",
        "事后补采诊断；采集项：现存日志；时间及范围见 receipt.json",
      ),
    ).toEqual({
      captureItem: "现存日志",
      description: "事后补采诊断；时间及范围见 receipt.json",
    });
    expect(
      splitPostCaptureAttachmentDescription(
        "post-capture-current-screen.png",
        "事后补采当前截图；采集于 2026-10-10T10:46:19Z",
      ),
    ).toEqual({
      captureItem: "当前截图",
      description: "事后补采当前截图；采集于 2026-10-10T10:46:19Z",
    });
    expect(
      splitPostCaptureAttachmentDescription("manual.png", "用户导入文件"),
    ).toEqual({ captureItem: "", description: "用户导入文件" });
  });

  it.each([
    ["功能", ["ui", "app", "activities"]],
    ["UI", ["ui", "display", "activities"]],
    ["多语言", ["ui", "locale", "display"]],
    ["动态过程", ["ui", "app", "activities"]],
    ["崩溃", ["crash", "exit", "app", "activities"]],
    [
      "ANR",
      ["crash", "exit", "app", "activities", "display", "memory", "bugreport"],
    ],
    ["广告", ["app", "activities"]],
    ["性能", ["memory", "frames", "cpu"]],
    ["网络", ["network"]],
    ["数据", ["storage", "app"]],
  ] as const)("%s 保留通用证据并增加专项", (kind, extra) => {
    const selected = recommendedCaptureItems(kind);
    expect(selected).toEqual(["logs", "screenshot", ...extra]);
    expect(new Set(selected).size).toBe(selected.length);
    expect(
      selected.every((id) => captureItems.some((item) => item.id === id)),
    ).toBe(true);
  });
  it("重试仅包含未完整获取项，恢复中断任务的等待项可重试", () => {
    const job = {
      status: "partial",
      items: {
        environment: { status: "complete" },
        logs: { status: "partial" },
        screenshot: { status: "failed" },
        ui: { status: "unavailable" },
        memory: { status: "pending" },
        frames: { status: "running" },
        exit: { status: "interrupted" },
      },
    } as unknown as PostCaptureJob;
    expect(retryableItems(job)).toEqual(["logs", "screenshot", "ui", "exit"]);
    expect(retryableItems({ ...job, status: "interrupted" })).toEqual([
      "logs",
      "screenshot",
      "ui",
      "exit",
      "memory",
      "frames",
    ]);
  });
  it("多次补采按项目合并为最新结果，已成功项不再默认重复勾选", () => {
    const older = {
      id: "older",
      status: "partial",
      items: {
        logs: { status: "complete", attachments: ["old-log"] },
        screenshot: { status: "failed", attachments: [] },
      },
    } as unknown as PostCaptureJob;
    const newer = {
      id: "newer",
      status: "complete",
      items: {
        screenshot: { status: "complete", attachments: ["new-screen"] },
      },
    } as unknown as PostCaptureJob;
    const latest = latestCaptureResults([newer, older]);
    expect(latest.map((entry) => [entry.itemId, entry.job.id])).toEqual([
      ["logs", "older"],
      ["screenshot", "newer"],
    ]);
    expect(recommendedUncollectedItems("功能", [newer, older])).toEqual([
      "ui",
      "app",
      "activities",
    ]);
  });
});

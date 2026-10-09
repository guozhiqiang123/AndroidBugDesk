// Run with PLAYWRIGHT_MODULE pointing at an installed Playwright index.mjs.
// All IPC and devices are mocks. The temporary Vite server is stopped before exit.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { spawn } from "node:child_process";
import assert from "node:assert/strict";
const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const output = path.resolve(process.env.QA_OUTPUT || "artifacts/gzq49/browser");
fs.mkdirSync(output, { recursive: true });
const log = fs.openSync(path.join(output, "vite.log"), "w");
const server = spawn(
  process.execPath,
  [
    "node_modules/vite/bin/vite.js",
    "--host",
    "127.0.0.1",
    "--port",
    "14749",
    "--strictPort",
  ],
  { stdio: ["ignore", log, log] },
);
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (server.exitCode !== null) throw Error("Vite exited");
    try {
      if ((await fetch("http://127.0.0.1:14749")).ok) {
        ready = true;
        break;
      }
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.ok(ready);
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROME_PATH
      ? { executablePath: process.env.CHROME_PATH }
      : {}),
  });
  const page = await browser.newPage({
    viewport: { width: 1280, height: 900 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route("**/*", (r) =>
    new URL(r.request().url()).hostname === "127.0.0.1"
      ? r.continue()
      : r.abort(),
  );
  const fixture = JSON.parse(
    fs.readFileSync("tests/fixtures/valid-draft.json"),
  );
  const metadata = JSON.parse(fs.readFileSync("tests/fixtures/metadata.json"));
  await page.addInitScript(
    ({ fixture, metadata }) => {
      const clone = (v) => structuredClone(v);
      fixture.sessionId = "00000000-0000-4000-8000-000000000009";
      const q = (window.__qa = {
        authorized: false,
        user: null,
        profile: metadata.profile,
        metadata,
        drafts: [fixture],
        calls: [],
        captures: [
          {
            id: "window",
            draftId: fixture.id,
            kind: "log-window",
            status: "pending",
            window: {
              markedAt: fixture.event.occurredAt,
              dueAt: fixture.event.occurredAt,
            },
          },
        ],
        failReport: false,
      });
      window.isTauri = true;
      const draft = () => q.drafts[0];
      const attached = (name, role, source, parent = null) => ({
        ...clone(fixture.attachments[0]),
        id: crypto.randomUUID(),
        name,
        role,
        source,
        derivedFrom: parent,
        confirmed: false,
      });
      window.__TAURI_INTERNALS__ = {
        convertFileSrc: () => "",
        invoke: async (cmd, args = {}) => {
          q.calls.push({ cmd });
          if (cmd === "bootstrap")
            return clone({
              ...q,
              adbPath: "/mock/adb",
              submissions: [],
              sessions: [],
            });
          if (cmd === "login") {
            q.authorized = true;
            q.user = {
              id: "mock-user",
              name: "模拟姓名",
              nickName: "模拟昵称",
              lastOrganization: "test-org",
            };
            return clone({ status: "valid", user: q.user, message: "mock" });
          }
          if (cmd === "clear_auth") {
            q.authorized = false;
            q.user = null;
            return;
          }
          if (cmd === "list_organizations")
            return [{ id: "test-org", name: "模拟组织" }];
          if (cmd === "list_projects")
            return [{ id: "test-project", name: "模拟项目" }];
          if (cmd === "load_metadata") return clone(q.metadata);
          if (cmd === "new_draft") return clone(draft());
          if (cmd === "load_selection_memory")
            return clone({
              workitemTypeId: fixture.workitemTypeId,
              assignedTo: fixture.assignedTo,
              versions: fixture.versions,
              business: fixture.business,
              customFields: fixture.customFields,
            });
          if (cmd === "remember_selections") return;
          if (cmd === "save_draft") {
            q.drafts[0] = {
              ...clone(args.draft),
              attachments: draft().attachments,
            };
            return clone(draft());
          }
          if (cmd === "capture_bugreport") {
            const record = {
              id: crypto.randomUUID(),
              draftId: draft().id,
              kind: "bugreport",
              status: q.failReport ? "failed" : "complete",
            };
            q.captures.push(record);
            if (q.failReport) {
              record.error = "设备断线，系统报告未完成（模拟）";
              draft().collectionErrors.push(record.error);
            } else
              draft().attachments.push(
                attached(
                  "system-report.zip",
                  "report",
                  "ADB 按需系统报告；未脱敏",
                ),
              );
            return clone(draft());
          }
          if (cmd === "redact_log") {
            const original = draft().attachments.find(
              (a) => a.id === args.attachmentId,
            );
            const copy = attached(
              "redacted.log",
              "log",
              "文本派生副本：仅按提供的精确文本替换；未保证完整脱敏",
              original.id,
            );
            const receipts = draft()
              .attachments.filter(
                (a) => a.role === "report" && a.derivedFrom === original.id,
              )
              .map((a) => ({
                ...clone(a),
                id: crypto.randomUUID(),
                derivedFrom: copy.id,
                confirmed: false,
              }));
            for (const a of draft().attachments)
              if (a.id === original.id || a.derivedFrom === original.id)
                a.confirmed = false;
            draft().attachments.push(
              copy,
              attached(
                "text-receipt.zip",
                "report",
                "文本替换记录（请核对副本）",
                copy.id,
              ),
              ...receipts,
            );
            return clone(draft());
          }
          if (cmd === "confirm_attachment") {
            const a = draft().attachments.find(
              (a) => a.id === args.attachmentId,
            );
            const owner =
              a.role === "report" && a.derivedFrom ? a.derivedFrom : a.id;
            a.coverageStart = args.coverageStart;
            a.coverageEnd = args.coverageEnd;
            for (const row of draft().attachments)
              if (
                row.id === owner ||
                (row.role === "report" && row.derivedFrom === owner)
              )
                row.confirmed = args.confirmed;
            return clone(draft());
          }
          if (cmd === "remove_attachment") {
            const a = draft().attachments.find(
              (a) => a.id === args.attachmentId,
            );
            if (
              a.role === "report" &&
              a.derivedFrom &&
              draft().attachments.some((parent) => parent.id === a.derivedFrom)
            )
              throw "来源回执须与关联副本一起保留；请移除关联副本以成组移除";
            draft().attachments = draft().attachments.filter(
              (a) =>
                a.id !== args.attachmentId &&
                !(a.role === "report" && a.derivedFrom === args.attachmentId),
            );
            return clone(draft());
          }
          if (cmd === "validate_draft") return [];
          if (cmd === "read_log") return "mock log secret@example.test";
          if (cmd === "list_remote_history") {
            if (q.authFailure) throw "[授权失效] HTTP 401";
            return { items: [], page: 1, hasMore: false };
          }
          throw Error("Unexpected IPC " + cmd);
        },
      };
    },
    { fixture, metadata },
  );
  await page.goto("http://127.0.0.1:14749");
  await page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  assert.equal(await page.locator("input[type=password]").inputValue(), "");
  assert.deepEqual(
    await page.evaluate(() => window.__qa.calls.map((c) => c.cmd)),
    ["bootstrap"],
  );
  await page.locator("input[type=password]").fill("mock-only");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator("#field-title input").waitFor();
  assert.equal(
    await page.locator(".user-profile strong").innerText(),
    "模拟昵称",
  );
  const preview = page.getByRole("button", {
    name: "进入提交预览",
    exact: true,
  });
  assert.ok(await preview.isDisabled(), "pending evidence blocks preview");
  const title = page.locator("#field-title input");
  await title.fill("尚未保存的人工编辑不能被刷新丢弃");
  await page.evaluate(() => {
    const q = window.__qa,
      d = q.drafts[0],
      raw = d.attachments[0];
    const windowLog = {
      ...raw,
      id: crypto.randomUUID(),
      name: "window.log",
      source: "ADB 问题日志窗口派生副本；未脱敏",
      derivedFrom: raw.id,
      confirmed: false,
    };
    const receipt = {
      ...raw,
      id: crypto.randomUUID(),
      name: "range-receipt.zip",
      role: "report",
      source: "日志窗口范围与缺口记录",
      derivedFrom: windowLog.id,
      confirmed: false,
    };
    d.attachments.push(windowLog, receipt);
    q.captures[0].status = "complete";
    q.captures[0].summary = {
      rangeCovered: false,
      selectedRecords: 12,
      coverageStart: raw.coverageStart,
      coverageEnd: raw.coverageEnd,
      clockNote: "设备与电脑时钟估算（模拟）",
      limitations: ["初始缓冲区连续性不确定"],
      gaps: [{ reason: "模拟断线缺口" }],
    };
  });
  await page.locator(".attachment").filter({ hasText: "window.log" }).waitFor();
  assert.equal(await title.inputValue(), "尚未保存的人工编辑不能被刷新丢弃");
  const status = page.locator(".capture-status").first();
  await status.getByText("查看时间依据与缺口").click();
  assert.match(await status.innerText(), /初始缓冲区连续性不确定/);
  assert.match(await status.innerText(), /模拟断线缺口/);
  await page.getByRole("button", { name: "采集系统报告", exact: true }).click();
  await page
    .locator(".attachment")
    .filter({ hasText: "system-report.zip" })
    .waitFor();
  await page.evaluate(() => (window.__qa.failReport = true));
  await page.getByRole("button", { name: "采集系统报告", exact: true }).click();
  await page
    .locator(".capture-status")
    .filter({ hasText: "设备断线，系统报告未完成" })
    .waitFor();
  assert.equal(
    await page
      .locator(".attachment")
      .filter({ hasText: "system-report.zip" })
      .count(),
    1,
  );
  const windowCard = page
    .locator(".attachment")
    .filter({ hasText: "window.log" });
  await windowCard.getByText("生成脱敏副本", { exact: true }).click();
  await windowCard.locator("input[type=password]").fill("secret@example.test");
  await windowCard
    .getByRole("button", { name: "生成副本", exact: true })
    .click();
  const redacted = page
    .locator(".attachment")
    .filter({ hasText: "redacted.log" });
  await redacted.waitFor();
  assert.equal(
    await windowCard.locator("input[type=password]").inputValue(),
    "",
  );
  await redacted.locator("input[type=checkbox]").check();
  await page.waitForFunction(() => {
    const d = window.__qa.drafts[0],
      a = d.attachments.find((a) => a.name === "redacted.log");
    return (
      a?.confirmed &&
      d.attachments
        .filter((r) => r.derivedFrom === a.id)
        .every((r) => r.confirmed)
    );
  });
  assert.equal(
    await windowCard.locator("input[type=checkbox]").isChecked(),
    false,
  );
  const textReceipt = page
    .locator(".attachment")
    .filter({ hasText: "text-receipt.zip" });
  await textReceipt
    .getByRole("button", { name: "从草稿移除", exact: true })
    .click();
  await page
    .getByRole("alert")
    .filter({ hasText: "来源回执须与关联副本一起保留" })
    .waitFor();
  assert.equal(await textReceipt.count(), 1);
  await textReceipt.locator("input[type=checkbox]").uncheck();
  await page.waitForFunction(
    () =>
      !window.__qa.drafts[0].attachments.find((a) => a.name === "redacted.log")
        .confirmed,
  );
  assert.equal(
    await redacted.locator("input[type=checkbox]").isChecked(),
    false,
  );
  await redacted.locator("input[type=checkbox]").check();
  await page.screenshot({
    path: path.join(output, "evidence-mock.png"),
    fullPage: true,
  });
  await redacted
    .getByRole("button", { name: "从草稿移除", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      !window.__qa.drafts[0].attachments.some(
        (a) => a.name === "redacted.log" || a.name === "text-receipt.zip",
      ),
  );
  assert.equal(await windowCard.count(), 1);
  await page.evaluate(() => {
    window.__qa.authFailure = true;
  });
  await page.locator("nav button").filter({ hasText: "历史提交" }).click();
  await page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  assert.equal(await page.locator(".user-profile").count(), 0);
  assert.deepEqual(errors, []);
  const calls = await page.evaluate(() => window.__qa.calls.map((c) => c.cmd));
  assert.equal(calls.filter((c) => c === "submit_bug").length, 0);
  assert.equal(calls.filter((c) => c === "check_auth").length, 0);
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "actual React browser clicks with mocked IPC; no real ADB or Yunxiao",
        scenarios: [
          "blank startup with bootstrap only",
          "nickname from mock user",
          "pending capture blocks preview",
          "local refresh preserves unsaved title",
          "range/time/uncertainty/gap display",
          "system report success and disconnect failure",
          "redaction clears input and deselects original",
          "paired confirmation and deselection",
          "standalone receipt removal blocked",
          "owner removal includes all paired receipts",
          "401 clears profile",
          "zero submit and startup validation calls",
        ],
        calls,
      },
      null,
      2,
    ),
  );
  console.log("12 renderer integration scenarios passed; no remote writes.");
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) {
    server.kill("SIGTERM");
    await new Promise((r) => server.once("exit", r));
  }
  fs.closeSync(log);
}

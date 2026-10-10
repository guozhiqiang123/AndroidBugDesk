// Actual history UI using a fixture produced by the Rust bootstrap/copy test.
// Clipboard/browser requests are mocked; no real cloud writes or native actions.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import {
  startBrowserCheck,
  recordBrowserFailure,
} from "./browser-check-support.mjs";

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const output = path.resolve(
  process.env.QA_OUTPUT || "artifacts/ui-receipt-links/browser",
);
const fixture = JSON.parse(
  fs.readFileSync(process.env.BUG_LINKS_BROWSER_FIXTURE, "utf8"),
);
const server = await startBrowserCheck(output);
let browser;
try {
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
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) =>
    new URL(route.request().url()).hostname === "127.0.0.1"
      ? route.continue()
      : route.abort(),
  );
  await page.addInitScript((fixture) => {
    const clone = (value) => structuredClone(value);
    const q = (window.__qa = {
      ...clone(fixture.bootstrap),
      calls: [],
      copied: "",
    });
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      convertFileSrc: () => "",
      invoke: async (cmd, args = {}) => {
        q.calls.push({ cmd, args: clone(args) });
        if (cmd === "bootstrap") return clone({ ...q, rememberLogin: false });
        if (cmd === "login") {
          q.authorized = true;
          q.user = {
            id: "mock-user",
            name: "模拟用户",
            lastOrganization: q.profile.organizationId,
          };
          return clone({ status: "valid", user: q.user, message: "mock" });
        }
        if (cmd === "list_organizations")
          return [{ id: q.profile.organizationId, name: "模拟组织" }];
        if (cmd === "list_projects")
          return [{ id: q.profile.projectId, name: "模拟项目" }];
        if (cmd === "load_project_settings")
          return {
            entries: [
              {
                projectId: q.profile.projectId,
                enabled: true,
                packageName: "",
              },
            ],
          };
        if (cmd === "load_metadata") return clone(fixture.metadata);
        if (cmd === "new_draft") {
          const draft = {
            ...clone(fixture.draft),
            id: crypto.randomUUID(),
            sessionId: null,
          };
          q.drafts.push(draft);
          q.draftScopes[draft.id] = {
            organizationId: q.profile.organizationId,
            projectId: q.profile.projectId,
          };
          return clone(draft);
        }
        if (cmd === "load_selection_memory") return {};
        if (cmd === "remember_selections") return;
        if (cmd === "save_draft") return clone(args.draft);
        if (cmd === "list_remote_history")
          return { items: [], page: 1, hasMore: false };
        if (cmd === "copy_receipt") {
          q.copied = args.repair ? fixture.copiedRepair : fixture.copiedUrl;
          return;
        }
        if (cmd === "open_receipt") return;
        throw Error("Unexpected IPC " + cmd);
      },
    };
  }, fixture);
  await page.goto(server.baseUrl);
  await page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  await page.locator('input[type="password"]').fill("mock-only");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator("#field-title input").waitFor();
  await page.getByRole("button", { name: "历史提交", exact: true }).click();
  await page
    .getByRole("heading", { name: "本机提交记录", exact: true })
    .waitFor();
  const card = page.locator(".submission").filter({ hasText: "SKII-608" });
  assert.equal(await card.count(), 1);
  assert.equal(
    await card.getByRole("button", { name: "打开云效", exact: true }).count(),
    1,
  );
  await card.getByRole("button", { name: "复制链接", exact: true }).click();
  await page.waitForFunction(
    () =>
      window.__qa.copied === "https://devops.aliyun.com/projex/bug/SKII-608",
  );
  await card
    .getByRole("button", { name: "复制修复任务输入", exact: true })
    .click();
  await page.waitForFunction(() =>
    window.__qa.copied.includes(
      "链接：https://devops.aliyun.com/projex/bug/SKII-608",
    ),
  );
  const copied = await page.evaluate(() => window.__qa.copied);
  assert.equal(copied, fixture.copiedRepair);
  assert.ok(!copied.includes("API 未返回"));
  await card.getByRole("button", { name: "打开云效", exact: true }).click();
  const actions = await page.evaluate(() =>
    window.__qa.calls.filter((call) =>
      ["copy_receipt", "open_receipt"].includes(call.cmd),
    ),
  );
  assert.deepEqual(actions, [
    { cmd: "copy_receipt", args: { id: fixture.draft.id, repair: false } },
    { cmd: "copy_receipt", args: { id: fixture.draft.id, repair: true } },
    { cmd: "open_receipt", args: { id: fixture.draft.id } },
  ]);
  assert.deepEqual(errors, []);
  assert.equal(
    await page.evaluate(() =>
      window.__qa.calls.some((call) => call.cmd === "submit_bug"),
    ),
    false,
  );
  await server.verifySource();
  await card.screenshot({ path: path.join(output, "history-bug-link.png") });
  fs.writeFileSync(path.join(output, "copied-repair.txt"), copied);
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "Actual React history with Rust-generated old-receipt fixture; mocked IPC only",
        scenarios: [
          "old submission exposes open/copy actions",
          "copy link uses serialNumber route",
          "repair input includes the same link and original workitem ID",
          "open request targets the same stored submission",
        ],
      },
      null,
      2,
    ) + "\n",
  );
  console.log(
    "4 history-link scenarios passed; no real cloud, clipboard or browser open.",
  );
} catch (error) {
  await recordBrowserFailure(output, error, browser);
  throw error;
} finally {
  await browser?.close();
  await server.close();
}

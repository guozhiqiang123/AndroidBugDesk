import { readFile, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import assert from "node:assert/strict";
import {
  startBrowserCheck,
  recordBrowserFailure,
} from "./browser-check-support.mjs";

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const output = resolve(
  process.env.QA_OUTPUT ?? "artifacts/ui-attachment-tools-20261010/browser",
);
const server = await startBrowserCheck(output);
let browser;
try {
  browser = await chromium.launch({ headless: true });
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
  const draft = JSON.parse(
    await readFile("tests/fixtures/valid-draft.json", "utf8"),
  );
  const metadata = JSON.parse(
    await readFile("tests/fixtures/metadata.json", "utf8"),
  );
  const image = {
    ...draft.attachments[0],
    id: "00000000-0000-4000-8000-000000000071",
    name: "a-very-long-picture-name-测试附件.png",
    role: "image",
    mime: "image/png",
    confirmed: false,
  };
  const sample = {
    ...image,
    id: "00000000-0000-4000-8000-000000000072",
    name: "测试数据.json",
    role: "sample",
    mime: "application/octet-stream",
  };
  draft.attachments = [
    { ...draft.attachments[0], confirmed: false },
    image,
    sample,
  ];
  draft.sessionId = null;
  await page.addInitScript(
    ({ draft, metadata }) => {
      window.isTauri = true;
      const q = (window.__qa = {
        draft,
        metadata,
        calls: [],
        failPreview: false,
      });
      const clone = (value) => structuredClone(value);
      const bootstrap = () => ({
        profile: metadata.profile,
        metadata,
        authorized: true,
        rememberLogin: true,
        user: { id: "mock-user", nickname: "模拟用户" },
        adbPath: null,
        drafts: [q.draft],
        draftScopes: {
          [q.draft.id]: {
            organizationId: metadata.profile.organizationId,
            projectId: metadata.profile.projectId,
          },
        },
        submissions: [],
        sessions: [],
        captures: [],
      });
      window.__TAURI_INTERNALS__ = {
        invoke: async (cmd, args = {}) => {
          q.calls.push({ cmd, args: clone(args) });
          if (cmd === "bootstrap") return clone(bootstrap());
          if (cmd === "check_auth")
            return { status: "valid", user: bootstrap().user };
          if (cmd === "list_organizations")
            return [{ id: metadata.profile.organizationId, name: "模拟组织" }];
          if (cmd === "list_projects")
            return [{ id: metadata.profile.projectId, name: "模拟项目" }];
          if (cmd === "load_project_settings")
            return {
              entries: [
                {
                  projectId: metadata.profile.projectId,
                  enabled: true,
                  packageName: q.draft.app.packageName,
                },
              ],
            };
          if (cmd === "load_metadata") return clone(metadata);
          if (cmd === "new_draft") return clone(q.draft);
          if (cmd === "load_selection_memory") return {};
          if (cmd === "save_draft") {
            const attachments = clone(q.draft.attachments);
            q.draft = clone(args.draft);
            q.draft.attachments = attachments;
            return clone(q.draft);
          }
          if (cmd === "save_attachment_coverage") {
            const a = q.draft.attachments.find(
              (a) => a.id === args.attachmentId,
            );
            a.coverageStart = args.coverageStart;
            a.coverageEnd = args.coverageEnd;
            return clone(q.draft);
          }
          if (cmd === "remove_attachment") {
            q.draft.attachments = q.draft.attachments.filter(
              (a) => a.id !== args.attachmentId,
            );
            return clone(q.draft);
          }
          if (cmd === "remove_all_attachments") {
            q.draft.attachments = [];
            q.draft.previewConfirmed = false;
            return clone(q.draft);
          }
          if (cmd === "preview_attachment") {
            if (q.failPreview) throw "模拟系统应用打开失败";
            return;
          }
          if (cmd === "validate_draft") return [];
          if (cmd === "remember_selections") return;
          throw new Error(`Unexpected IPC: ${cmd}`);
        },
      };
    },
    { draft, metadata },
  );
  await page.goto(server.baseUrl);
  await page.locator(".attachment").first().waitFor();
  assert.equal(
    await page.locator(".attachment input[type=checkbox]").count(),
    0,
  );
  assert.equal(
    await page.getByText("已预览，确认上传", { exact: true }).count(),
    0,
  );
  assert.equal(
    await page.getByRole("button", { name: "从草稿移除", exact: true }).count(),
    0,
  );
  const removeAll = page.getByRole("button", {
    name: "移除所有附件",
    exact: true,
  });
  await removeAll.waitFor();
  assert.ok(
    await removeAll.evaluate(
      (button) =>
        getComputedStyle(button).backgroundColor !== "rgb(255, 255, 255)",
    ),
  );
  for (const width of [980, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const actionGeometry = await page
      .locator(".attachment-actions")
      .evaluate((row) => {
        const buttons = [
          ...row.querySelectorAll(":scope > .action-item > button"),
        ];
        const boxes = buttons.map((button) => button.getBoundingClientRect());
        return {
          labels: buttons.map((button) => button.textContent?.trim()),
          aligned:
            boxes.every((box) => Math.abs(box.top - boxes[0].top) < 1) &&
            boxes.every((box) => Math.abs(box.width - boxes[0].width) < 1) &&
            row.scrollWidth <= row.clientWidth,
        };
      });
    assert.equal(actionGeometry.labels.length, 4);
    assert.equal(actionGeometry.labels[0], "设备环境快照");
    assert.ok(actionGeometry.labels[1]?.startsWith("补采证据"));
    assert.equal(actionGeometry.labels[2], "导入附件");
    assert.equal(actionGeometry.labels[3], "移除所有附件");
    assert.ok(actionGeometry.aligned, `Action layout at ${width}px`);
    const geometry = await page.locator(".attachment").evaluateAll((cards) =>
      cards.map((card) => {
        const body = card
          .querySelector(".attachment-body")
          .getBoundingClientRect();
        const tools = card
          .querySelector(".attachment-tools")
          .getBoundingClientRect();
        const buttons = [
          ...card.querySelectorAll(".attachment-tools button"),
        ].map((button) => button.getBoundingClientRect());
        return (
          tools.right <= body.right + 1 &&
          Math.abs(tools.right - body.right) < 1 &&
          buttons[0].bottom <= buttons[1].top &&
          Math.abs(buttons[0].left - buttons[1].left) < 1 &&
          Math.abs(buttons[0].width - buttons[1].width) < 1 &&
          card.scrollWidth <= card.clientWidth
        );
      }),
    );
    assert.ok(geometry.every(Boolean), `Card layout at ${width}px`);
    await page
      .locator(".attachment")
      .filter({ hasText: image.name })
      .screenshot({ path: join(output, `attachment-${width}.png`) });
  }
  const card = page.locator(".attachment").filter({ hasText: image.name });
  await card.getByRole("button", { name: "预览", exact: true }).click();
  await page.waitForFunction(() =>
    window.__qa.calls.some((call) => call.cmd === "preview_attachment"),
  );
  await page.evaluate(() => (window.__qa.failPreview = true));
  await card.getByRole("button", { name: "预览", exact: true }).click();
  await card.getByText("模拟系统应用打开失败").waitFor();
  assert.ok(
    await card
      .locator(".action-item")
      .evaluate(
        (node) =>
          node.querySelector("small").getBoundingClientRect().top >=
          node.querySelector("button").getBoundingClientRect().bottom,
      ),
  );
  await page.evaluate(() => (window.__qa.failPreview = false));
  await card.getByRole("button", { name: "预览", exact: true }).click();
  await page.waitForFunction(
    () => !document.querySelector(".attachment-tools .error-text"),
  );
  const log = page.locator(".attachment").filter({ hasText: "logcat.log" });
  await log.getByRole("button", { name: "保存覆盖时间", exact: true }).click();
  const coverageArgs = await page.evaluate(
    () =>
      window.__qa.calls.find((call) => call.cmd === "save_attachment_coverage")
        .args,
  );
  assert.deepEqual(Object.keys(coverageArgs).sort(), [
    "attachmentId",
    "coverageEnd",
    "coverageStart",
    "id",
  ]);
  await page
    .locator(".attachment")
    .filter({ hasText: sample.name })
    .getByRole("button", { name: "移除", exact: true })
    .click();
  await page.waitForFunction(
    () => document.querySelectorAll(".attachment").length === 2,
  );
  await page.getByRole("button", { name: "进入提交预览", exact: true }).click();
  await page.getByText("将上传", { exact: true }).waitFor();
  assert.ok(
    (
      await page
        .getByText("将上传", { exact: true })
        .locator("xpath=following-sibling::dd[1]")
        .innerText()
    ).includes(image.name),
  );
  assert.equal(
    await page.evaluate(() =>
      window.__qa.calls.some(
        (call) =>
          call.cmd === "confirm_attachment" || call.cmd === "submit_bug",
      ),
    ),
    false,
  );
  await page.getByRole("button", { name: "关闭提交预览", exact: true }).click();
  await removeAll.click();
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".attachment").length === 0 &&
      ![...document.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "移除所有附件",
      ),
  );
  assert.deepEqual(
    await page.evaluate(() => window.__qa.draft.attachments),
    [],
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter(
          (call) => call.cmd === "remove_all_attachments",
        ).length,
    ),
    1,
  );
  assert.deepEqual(errors, []);
  await server.verifySource();
  await writeFile(
    join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        scenarios: [
          "no attachment confirmation controls",
          "right-side stacked actions at three widths",
          "system preview and local failure/retry",
          "coverage save accepts no confirmation flag",
          "remove action changes draft",
          "legacy false confirmation still appears in upload preview",
          "remove-all action is blue, clears the draft, and hides when empty",
        ],
        mode: "Actual React with mocked IPC; no cloud, USB or system viewer launch",
      },
      null,
      2,
    ) + "\n",
  );
} catch (error) {
  await recordBrowserFailure(output, error, browser);
  throw error;
} finally {
  await browser?.close();
  await server.close();
}

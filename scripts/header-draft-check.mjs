// Isolated renderer checks. Only mocked IPC; no installed app or real data is touched.
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
  process.env.QA_OUTPUT || "artifacts/ui-header-drafts/browser",
);
fs.mkdirSync(output, { recursive: true });
const server = await startBrowserCheck(output);
let browser;
checks: try {
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
  const fixture = JSON.parse(
    fs.readFileSync("tests/fixtures/valid-draft.json"),
  );
  const metadata = JSON.parse(fs.readFileSync("tests/fixtures/metadata.json"));
  await page.addInitScript(
    ({ fixture, metadata }) => {
      const clone = (v) => structuredClone(v);
      const q = (window.__qa = {
        profile: clone(metadata.profile),
        metadata: clone(metadata),
        authorized: false,
        user: null,
        drafts: [],
        draftScopes: {},
        submissions: [],
        sessions: [],
        captures: [],
        calls: [],
        failDeletion: false,
      });
      const foreign = {
        ...clone(fixture),
        id: crypto.randomUUID(),
        title: "其他项目的草稿",
        sessionId: null,
      };
      q.drafts.push(foreign);
      q.draftScopes[foreign.id] = {
        organizationId: q.profile.organizationId,
        projectId: "other-project",
      };
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = {
        convertFileSrc: () => "",
        invoke: async (cmd, args = {}) => {
          q.calls.push({ cmd, id: args.id, ids: args.ids });
          if (cmd === "bootstrap")
            return clone({ ...q, rememberLogin: false, adbPath: "/mock/adb" });
          if (cmd === "login") {
            q.authorized = true;
            q.user = {
              id: "mock-user",
              name: "模拟用户",
              lastOrganization: q.profile.organizationId,
            };
            return clone({ status: "valid", user: q.user, message: "mock" });
          }
          if (cmd === "clear_auth") {
            q.authorized = false;
            q.user = null;
            return;
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
                  packageName: "com.example.mock",
                },
              ],
            };
          if (cmd === "select_project") return clone(q.profile);
          if (cmd === "load_metadata") return clone(q.metadata);
          if (cmd === "new_draft") {
            const d = {
              ...clone(fixture),
              id: crypto.randomUUID(),
              title: "",
              sessionId: null,
              createdAt: new Date().toISOString(),
              updatedAt: new Date().toISOString(),
            };
            q.drafts.unshift(d);
            q.draftScopes[d.id] = {
              organizationId: q.profile.organizationId,
              projectId: q.profile.projectId,
            };
            return clone(d);
          }
          if (cmd === "save_draft") {
            const index = q.drafts.findIndex((d) => d.id === args.draft.id);
            if (index < 0) throw "草稿不存在";
            q.drafts[index] = {
              ...clone(args.draft),
              attachments: q.drafts[index].attachments,
            };
            return clone(q.drafts[index]);
          }
          if (cmd === "load_selection_memory") return {};
          if (cmd === "remember_selections") return;
          if (cmd === "list_remote_history")
            return { items: [], page: 1, hasMore: false };
          if (cmd === "delete_draft" || cmd === "delete_drafts") {
            const ids = cmd === "delete_drafts" ? args.ids : [args.id];
            if (q.failDeletion) throw "模拟删除失败";
            if (q.submissions.some((s) => ids.includes(s.draftId)))
              throw "证据已固定，不能删除";
            if (
              q.captures.some(
                (t) => ids.includes(t.draftId) && t.status === "pending",
              )
            )
              throw "证据正在采集，不能删除";
            for (const id of ids) {
              const scope = q.draftScopes[id];
              if (
                scope?.projectId !== q.profile.projectId ||
                scope?.organizationId !== q.profile.organizationId
              )
                throw "不能删除其他项目草稿";
            }
            q.drafts = q.drafts.filter((d) => !ids.includes(d.id));
            ids.forEach((id) => delete q.draftScopes[id]);
            q.captures = q.captures.filter((t) => !ids.includes(t.draftId));
            return;
          }
          throw Error("Unexpected IPC " + cmd);
        },
      };
    },
    { fixture, metadata },
  );
  await page.goto(server.baseUrl);
  await page.getByRole("tab", { name: "填写 PAT", exact: true }).click();
  await page.locator("input[type=password]").fill("mock-only");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator("#field-title input").waitFor();
  const header = page.locator(".page-header");
  const project = header.getByRole("combobox", {
    name: "当前项目",
    exact: true,
  });
  const drafts = header.getByRole("combobox", {
    name: "当前草稿",
    exact: true,
  });
  assert.equal(await project.count(), 1);
  assert.equal(await drafts.count(), 1);
  assert.equal(
    await page.locator(".project-selector,.draft-switch").count(),
    0,
  );
  assert.doesNotMatch(await drafts.innerText(), /当前值不可用/);
  const first = await page.evaluate(() => window.__qa.drafts[0].id);
  if (process.env.QA_SCOPE === "submitted-drafts") {
    await page.locator("#field-title input").fill("本次完成草稿");
    const { completed, editable } = await page.evaluate(() => {
      const q = window.__qa;
      const completed = {
        ...structuredClone(q.drafts[0]),
        id: crypto.randomUUID(),
        title: "已提交历史记录",
        sessionId: null,
      };
      const editable = {
        ...structuredClone(q.drafts[0]),
        id: crypto.randomUUID(),
        title: "保留的待提交草稿",
        sessionId: null,
      };
      for (const draft of [completed, editable]) {
        q.drafts.push(draft);
        q.draftScopes[draft.id] = {
          organizationId: q.profile.organizationId,
          projectId: q.profile.projectId,
        };
      }
      q.submissions.push({
        draftId: completed.id,
        state: "submitted",
        identifier: "MOCK-1",
        uploads: [],
        evidence: { id: "immutable-mock-evidence" },
      });
      return { completed: completed.id, editable: editable.id };
    });
    await page
      .getByRole("button", { name: "刷新云效配置", exact: true })
      .click();
    await drafts.click();
    assert.equal(
      await header.locator(`[data-option-id="${completed}"]`).count(),
      0,
    );
    assert.equal(await page.getByRole("option").count(), 2);
    assert.equal(
      await header
        .getByRole("button", { name: "删除全部", exact: true })
        .isDisabled(),
      false,
    );
    await header.screenshot({
      path: path.join(output, "submitted-excluded.png"),
    });
    await drafts.press("Escape");
    await page.evaluate((id) => {
      window.__qa.submissions.push({
        draftId: id,
        state: "submitted",
        identifier: "MOCK-2",
        uploads: [],
        evidence: { id: "current-completed-evidence" },
      });
    }, first);
    await page
      .getByRole("button", { name: "刷新云效配置", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector("#field-title input")?.value ===
        "保留的待提交草稿",
    );
    assert.doesNotMatch(await drafts.innerText(), /当前值不可用|本次完成草稿/);
    const reloadHistory = async () => {
      await page.getByRole("button", { name: "历史提交", exact: true }).click();
      await page
        .locator(".card")
        .filter({
          has: page.getByRole("heading", { name: "本机提交记录", exact: true }),
        })
        .getByRole("button", { name: "刷新", exact: true })
        .click();
      await page.getByRole("button", { name: "新建提交", exact: true }).click();
    };
    for (const state of ["ready", "create_unknown", "partial"]) {
      await page.evaluate(
        ({ id, state }) => {
          const q = window.__qa;
          q.submissions = q.submissions.filter((s) => s.draftId !== id);
          q.submissions.push({
            draftId: id,
            state,
            uploads: [],
            evidence: { id: "unfinished-evidence" },
          });
        },
        { id: editable, state },
      );
      await reloadHistory();
      await drafts.click();
      assert.equal(
        await header.locator(`[data-option-id="${editable}"]`).count(),
        1,
      );
      assert.equal(
        await header.locator(`[data-option-id="${completed}"]`).count(),
        0,
      );
      assert.equal(
        await header
          .getByRole("button", { name: "删除全部", exact: true })
          .isDisabled(),
        true,
      );
      await drafts.press("Escape");
    }
    await page.evaluate((id) => {
      window.__qa.submissions = window.__qa.submissions.filter(
        (s) => s.draftId !== id,
      );
    }, editable);
    await reloadHistory();
    const preserved = await page.evaluate(
      (ids) => ({
        drafts: window.__qa.drafts.filter((d) => ids.includes(d.id)),
        submissions: structuredClone(window.__qa.submissions),
      }),
      [first, completed],
    );
    await drafts.click();
    await header.getByRole("button", { name: "删除全部", exact: true }).click();
    await page.getByRole("button", { name: "确认删除", exact: true }).click();
    await page
      .getByRole("heading", { name: "记录第一个问题", exact: true })
      .waitFor();
    assert.equal(await drafts.isDisabled(), true);
    const remaining = await page.evaluate(
      (ids) => ({
        drafts: window.__qa.drafts.filter((d) => ids.includes(d.id)),
        submissions: window.__qa.submissions,
      }),
      [first, completed],
    );
    assert.deepEqual(remaining, preserved);
    const deletions = await page.evaluate(() =>
      window.__qa.calls.filter((c) =>
        ["delete_draft", "delete_drafts"].includes(c.cmd),
      ),
    );
    assert.deepEqual(
      deletions.map((c) => c.ids ?? [c.id]),
      [[editable]],
    );
    await page.getByRole("button", { name: "历史提交", exact: true }).click();
    assert.equal(await page.locator(".submission").count(), 2);
    assert.equal(
      await page
        .getByRole("heading", { name: "已提交历史记录", exact: true })
        .count(),
      1,
    );
    assert.equal(
      await page
        .getByRole("heading", { name: "本次完成草稿", exact: true })
        .count(),
      1,
    );
    await page
      .locator(".submission")
      .first()
      .screenshot({ path: path.join(output, "history-preserved.png") });
    await page.getByRole("button", { name: "新建提交", exact: true }).click();
    await header
      .getByRole("button", { name: "＋ 新建问题", exact: true })
      .click();
    const last = await page.evaluate(() => window.__qa.drafts[0].id);
    await page.evaluate((id) => {
      window.__qa.submissions.push({
        draftId: id,
        state: "submitted",
        uploads: [],
        evidence: { id: "last-evidence" },
      });
    }, last);
    await reloadHistory();
    await page
      .getByRole("heading", { name: "记录第一个问题", exact: true })
      .waitFor();
    assert.equal(await drafts.isDisabled(), true);
    assert.doesNotMatch(await drafts.innerText(), /当前值不可用/);
    assert.deepEqual(errors, []);
    assert.equal(
      await page.evaluate(
        () => window.__qa.calls.filter((c) => c.cmd === "submit_bug").length,
      ),
      0,
    );
    await server.verifySource();
    const scenarios = [
      "submitted records are absent from the draft menu",
      "submitted history does not disable delete-all",
      "completing the current draft selects a remaining draft without a stale value",
      "ready/create_unknown/partial submissions remain visible and protected",
      "delete-all targets only remaining editable drafts",
      "completed draft contents and submission receipts remain unchanged",
      "history retains titles and completed submissions",
      "completing the final draft leaves the new-problem empty state",
    ];
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "Actual React with mocked submission states; no real cloud, deletion or installed data",
          scenarios,
          deletions,
        },
        null,
        2,
      ) + "\n",
    );
    console.log(`${scenarios.length} submitted-draft scenarios passed`);
    break checks;
  }
  const deleteItem = async (id) => {
    await drafts.click();
    return header.locator(
      `[data-option-id="${id}"] .custom-select-item-action`,
    );
  };
  assert.equal(await header.locator(".delete-draft-button").count(), 0);
  await drafts.click();
  const selectedRow = header.locator(
    '.custom-select-option[aria-selected="true"]',
  );
  assert.ok(
    await selectedRow.evaluate((row) => {
      const indicator = row.querySelector(".custom-select-indicator");
      return (
        indicator === row.firstElementChild && indicator.textContent === "✓"
      );
    }),
  );
  assert.equal(
    await header.getByRole("button", { name: "删除全部", exact: true }).count(),
    1,
  );
  assert.equal(
    await page.getByRole("option").filter({ hasText: "其他项目" }).count(),
    0,
  );
  await drafts.press("Escape");
  for (const width of [1280, 980]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => scrollTo(0, 0));
    const initial = await header.boundingBox();
    assert.ok(
      await header.evaluate((node) => {
        const box = node.getBoundingClientRect();
        return (
          [
            ...node.querySelectorAll('[role="combobox"],.delete-draft-button'),
          ].every((el) => {
            const r = el.getBoundingClientRect();
            return (
              r.left >= box.left &&
              r.right <= box.right + 1 &&
              r.bottom <= box.bottom + 1
            );
          }) && document.documentElement.scrollWidth <= innerWidth
        );
      }),
    );
    await page.evaluate(() => scrollTo(0, 600));
    await page.waitForFunction(() => scrollY >= 500);
    const scrolled = await header.boundingBox();
    assert.ok(Math.abs(scrolled.y - initial.y) <= 1);
    await header.screenshot({ path: path.join(output, `header-${width}.png`) });
  }
  await (await deleteItem(first)).click();
  await page.getByRole("dialog").waitFor();
  assert.ok(
    (await page.locator(".delete-draft-dialog").boundingBox()).width <= 440,
  );
  const deleteActions = await page
    .locator(".delete-draft-dialog > .actions")
    .evaluate((node) => {
      const actionBox = node.getBoundingClientRect();
      const buttons = [...node.querySelectorAll("button")].map((button) => {
        const box = button.getBoundingClientRect();
        return { left: box.left, right: box.right, width: box.width };
      });
      return { actionRight: actionBox.right, buttons };
    });
  assert.equal(deleteActions.buttons.length, 2);
  assert.ok(
    Math.abs(deleteActions.buttons[0].width - deleteActions.buttons[1].width) <
      1,
    "delete dialog buttons have equal widths",
  );
  assert.ok(
    Math.abs(deleteActions.actionRight - deleteActions.buttons[1].right) < 1,
    "delete dialog actions align to the right",
  );
  await page.getByRole("dialog").screenshot({
    path: path.join(output, "delete-confirmation.png"),
  });
  await page.getByRole("button", { name: "取消", exact: true }).click();
  assert.equal(
    await page.evaluate(
      () => window.__qa.calls.filter((c) => c.cmd === "delete_draft").length,
    ),
    0,
  );
  await page.locator("#field-title input").fill("待保存的第一个草稿");
  await header
    .getByRole("button", { name: "＋ 新建问题", exact: true })
    .click();
  await page.waitForFunction(
    (first) => window.__qa.drafts[0].id !== first,
    first,
  );
  const second = await page.evaluate(() => window.__qa.drafts[0].id);
  assert.equal(
    await page.evaluate(
      (first) => window.__qa.drafts.find((d) => d.id === first).title,
      first,
    ),
    "待保存的第一个草稿",
  );
  assert.doesNotMatch(await drafts.innerText(), /当前值不可用/);
  await drafts.click();
  await page
    .getByRole("option")
    .filter({ hasText: "待保存的第一个草稿" })
    .click();
  assert.equal(
    await page.locator("#field-title input").inputValue(),
    "待保存的第一个草稿",
  );
  await page.evaluate(() => (window.__qa.failDeletion = true));
  await (await deleteItem(first)).click();
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await header.getByText("模拟删除失败", { exact: true }).waitFor();
  assert.ok(
    await page.evaluate(
      (id) => window.__qa.drafts.some((d) => d.id === id),
      first,
    ),
  );
  await page.evaluate(() => (window.__qa.failDeletion = false));
  await (await deleteItem(first)).click();
  await page
    .getByRole("dialog")
    .screenshot({ path: path.join(output, "delete-confirm.png") });
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await page.waitForFunction(
    (id) => !window.__qa.drafts.some((d) => d.id === id),
    first,
  );
  assert.match(await drafts.innerText(), /未命名草稿/);
  await page.evaluate(
    (second) =>
      (window.__qa.captures = [
        { id: "pending", draftId: second, status: "pending" },
      ]),
    second,
  );
  await page.getByRole("button", { name: "刷新云效配置", exact: true }).click();
  assert.equal(await (await deleteItem(second)).isDisabled(), true);
  assert.equal(
    await header
      .getByRole("button", { name: "删除全部", exact: true })
      .isDisabled(),
    true,
  );
  await drafts.press("Escape");
  await page.evaluate((second) => {
    window.__qa.captures = [];
    window.__qa.submissions = [{ draftId: second, state: "create_unknown" }];
  }, second);
  await page.getByRole("button", { name: "刷新云效配置", exact: true }).click();
  assert.equal(await (await deleteItem(second)).isDisabled(), true);
  assert.equal(
    await header
      .getByRole("button", { name: "删除全部", exact: true })
      .isDisabled(),
    true,
  );
  await drafts.press("Escape");
  // Reload gives a fresh editable draft without touching the frozen one.
  await page.evaluate(() => (window.__qa.submissions = []));
  await header
    .getByRole("button", { name: "＋ 新建问题", exact: true })
    .click();
  const third = await page.evaluate(() => window.__qa.drafts[0].id);
  await page.locator("#field-title input").fill("删除其他草稿时保留当前编辑");
  await (await deleteItem(second)).click();
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await page.waitForFunction(
    (id) => !window.__qa.drafts.some((d) => d.id === id),
    second,
  );
  assert.equal(
    await page.locator("#field-title input").inputValue(),
    "删除其他草稿时保留当前编辑",
  );
  assert.equal(
    await page.evaluate(
      (id) => window.__qa.drafts.find((d) => d.id === id).title,
      third,
    ),
    "删除其他草稿时保留当前编辑",
  );
  assert.match(await drafts.innerText(), /删除其他草稿时保留当前编辑/);
  await header
    .getByRole("button", { name: "＋ 新建问题", exact: true })
    .click();
  await drafts.click();
  await header.getByRole("button", { name: "删除全部", exact: true }).click();
  await page
    .getByRole("heading", { name: "删除当前项目全部草稿？", exact: true })
    .waitFor();
  await page.getByRole("button", { name: "取消", exact: true }).click();
  assert.equal(
    await page.evaluate(
      () => window.__qa.calls.filter((c) => c.cmd === "delete_drafts").length,
    ),
    0,
  );
  await page.evaluate(() => {
    window.__qa.failDeletion = true;
  });
  await drafts.click();
  await drafts.press("End");
  await drafts.press("Enter");
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await header.getByText("模拟删除失败", { exact: true }).waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.drafts.filter((d) => d.title !== "其他项目的草稿").length,
    ),
    2,
  );
  await page.evaluate(() => {
    window.__qa.failDeletion = false;
  });
  await drafts.click();
  await header.getByRole("button", { name: "删除全部", exact: true }).click();
  await page
    .getByRole("dialog")
    .screenshot({ path: path.join(output, "delete-all-confirmation.png") });
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await page
    .getByRole("heading", { name: "记录第一个问题", exact: true })
    .waitFor();
  assert.equal(await drafts.isDisabled(), true);
  assert.ok(
    await page.evaluate(() =>
      window.__qa.drafts.some((d) => d.title === "其他项目的草稿"),
    ),
  );
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  assert.equal(await header.count(), 0);
  await page.locator("nav button").filter({ hasText: "历史提交" }).click();
  assert.equal(await header.count(), 0);
  assert.deepEqual(errors, []);
  const calls = await page.evaluate(() => window.__qa.calls);
  assert.equal(calls.filter((c) => c.cmd === "submit_bug").length, 0);
  const scenarios = [
    "project and draft selectors inside sticky header",
    "fresh active draft has a real selectable label",
    "other-project drafts stay hidden",
    "wide and narrow header fit and stay fixed",
    "deletion cancel performs no IPC mutation",
    "switching saves dirty edits",
    "deletion failure retains draft and shows local error",
    "deletion switches to remaining draft",
    "pending capture and frozen submission disable deletion",
    "last deletion leaves empty state and preserves foreign draft",
    "other tabs hide the header",
    "draft item actions and leading selection marker",
    "deleting another draft preserves current unsaved edits",
    "delete-all cancel and failure preserve every draft",
    "delete-all keyboard action clears only the current project",
  ];
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "actual React browser with mock IPC; no real credentials, cloud, device or installed data",
        scenarios,
        calls,
      },
      null,
      2,
    ),
  );
  console.log(`${scenarios.length} header/draft scenarios passed`);
  await server.verifySource();
} catch (error) {
  await recordBrowserFailure(output, error, browser);
  throw error;
} finally {
  try {
    await browser?.close();
  } finally {
    await server.close();
  }
}

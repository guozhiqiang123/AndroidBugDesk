// Real React UI with isolated mock IPC. No native data, cloud or credentials.
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
  process.env.QA_OUTPUT || "artifacts/ui-history-filters-20261010/browser",
);
const draft = JSON.parse(
  fs.readFileSync("tests/fixtures/valid-draft.json", "utf8"),
);
const metadata = JSON.parse(
  fs.readFileSync("tests/fixtures/metadata.json", "utf8"),
);
metadata.members = [
  { id: "u1", name: "郭志强" },
  { id: "u2", name: "验证者" },
  { id: "u3", name: "参与人" },
  { id: "u4", name: "抄送人" },
];
metadata.sprints = [{ id: "sp1", name: "当前迭代" }];
metadata.fields = [
  {
    id: "priority",
    name: "优先级",
    type: "SystemCustomField",
    format: "list",
    required: true,
    options: [{ id: "p2", name: "中" }],
  },
  {
    id: "severity",
    name: "严重程度",
    type: "SystemCustomField",
    format: "list",
    required: true,
    options: [{ id: "s2", name: "3—一般" }],
  },
  ...["verifier", "participants", "trackers"].map((id) => ({
    id,
    name: id,
    type: "Role",
    format: id === "verifier" ? "user" : "multiUser",
    required: false,
    options: [],
  })),
];
const server = await startBrowserCheck(output);
let browser;
const scenarios = [];
try {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROME_PATH
      ? { executablePath: process.env.CHROME_PATH }
      : {}),
  });
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1050 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route("**/*", (route) =>
    new URL(route.request().url()).hostname === "127.0.0.1"
      ? route.continue()
      : route.abort(),
  );
  await page.addInitScript(
    ({ draft, metadata }) => {
      const clone = (v) => structuredClone(v);
      const user = {
        id: "user-a",
        name: "模拟账号",
        lastOrganization: metadata.profile.organizationId,
      };
      const q = (window.__qa = {
        profile: clone(metadata.profile),
        metadata: clone(metadata),
        user,
        authorized: true,
        rememberLogin: true,
        drafts: [],
        draftScopes: {},
        submissions: [],
        sessions: [],
        captures: [],
        calls: [],
        pending: [],
        deferHistory: false,
        rejectHistory: "",
        emptyHistory: false,
        sparseHistory: false,
      });
      const add = (index, foreign = false) => {
        const d = {
          ...clone(draft),
          id: crypto.randomUUID(),
          sessionId: null,
          title: `本机问题 ${index}`,
          kind: index % 2 ? "功能" : "UI",
          assignedTo: "u1",
          sprint: "sp1",
          customFields: {
            priority: "p2",
            severity: "s2",
            verifier: "u2",
            participants: ["u3"],
            trackers: ["u4"],
          },
        };
        q.drafts.push(d);
        q.draftScopes[d.id] = {
          organizationId: q.profile.organizationId,
          projectId: foreign ? "other-project" : q.profile.projectId,
        };
        q.submissions.push({
          draftId: d.id,
          profile: {
            ...clone(q.profile),
            projectId: foreign ? "other-project" : q.profile.projectId,
          },
          state:
            index === 30
              ? "partial"
              : index === 29
                ? "create_unknown"
                : "submitted",
          identifier: `BUG-${index}`,
          workitemId: `work-${index}`,
          url: `https://devops.aliyun.com/projex/bug/BUG-${index}`,
          evidence: { id: `evidence-${index}`, files: [] },
          uploads: [],
          updatedAt: `2026-10-10T10:${String(index).padStart(2, "0")}:00Z`,
        });
      };
      for (let i = 1; i <= 30; i++) add(i);
      for (let i = 31; i <= 35; i++) add(i, true);
      const active = {
        ...clone(draft),
        id: crypto.randomUUID(),
        sessionId: null,
        title: "当前编辑中的问题",
        assignedTo: "u1",
      };
      q.drafts.push(active);
      q.draftScopes[active.id] = {
        organizationId: q.profile.organizationId,
        projectId: q.profile.projectId,
      };
      const item = (id, title) => ({
        id,
        identifier: id,
        title,
        status: "待处理",
        createdAt: "2026-10-10",
        updatedAt: "2026-10-10",
      });
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = {
        convertFileSrc: () => "",
        invoke: async (cmd, args = {}) => {
          q.calls.push({ cmd, args: clone(args) });
          if (cmd === "bootstrap")
            return clone({
              profile: q.profile,
              metadata: q.metadata,
              user: q.user,
              authorized: q.authorized,
              rememberLogin: q.rememberLogin,
              drafts: q.drafts,
              draftScopes: q.draftScopes,
              submissions: q.submissions,
              sessions: q.sessions,
              captures: q.captures,
            });
          if (cmd === "check_auth" || cmd === "login")
            return { status: "valid", user: clone(user), message: "mock" };
          if (cmd === "list_organizations")
            return [{ id: q.profile.organizationId, name: "模拟组织" }];
          if (cmd === "list_projects")
            return [
              { id: "test-project", name: "模拟验收项目" },
              { id: "other-project", name: "另一个项目" },
            ];
          if (cmd === "load_project_settings")
            return {
              entries: [
                {
                  projectId: "test-project",
                  enabled: true,
                  packageName: "com.example.mock",
                },
                {
                  projectId: "other-project",
                  enabled: true,
                  packageName: "com.example.other",
                },
              ],
            };
          if (cmd === "load_metadata") return clone(q.metadata);
          if (cmd === "select_project") {
            q.profile = {
              ...q.profile,
              projectId: args.projectId,
              name:
                args.projectId === "test-project"
                  ? "模拟验收项目"
                  : "另一个项目",
            };
            q.metadata.profile = clone(q.profile);
            return clone(q.profile);
          }
          if (cmd === "load_selection_memory") return {};
          if (cmd === "remember_selections") return;
          if (cmd === "save_draft") {
            const i = q.drafts.findIndex((d) => d.id === args.draft.id);
            q.drafts[i] = clone(args.draft);
            return clone(args.draft);
          }
          if (cmd === "new_draft") {
            const d = {
              ...clone(draft),
              id: crypto.randomUUID(),
              sessionId: null,
              title: "",
            };
            q.drafts.push(d);
            q.draftScopes[d.id] = {
              organizationId: q.profile.organizationId,
              projectId: q.profile.projectId,
            };
            return clone(d);
          }
          if (cmd === "copy_receipt" || cmd === "open_receipt") return;
          if (cmd === "submit_bug") {
            q.submissions.find((s) => s.draftId === args.id).state =
              "submitted";
            return;
          }
          if (cmd === "reconcile_submission") {
            q.submissions.find((s) => s.draftId === args.id).state = "partial";
            return;
          }
          if (cmd === "list_remote_history") {
            const failure = q.rejectHistory;
            const filtered = args.filters.kind.length > 0;
            const result = q.emptyHistory
              ? { items: [], page: 1, hasMore: false }
              : q.sparseHistory && args.page === 1
                ? { items: [], page: 3, hasMore: true }
                : filtered
                  ? {
                      items: [item("FILTER-1", "图片筛选命中")],
                      page: 1,
                      hasMore: false,
                    }
                  : args.page === 1
                    ? {
                        items: [
                          item("CLOUD-1", `${q.profile.name} 云效记录 1`),
                          item("CLOUD-2", "云效记录 2"),
                        ],
                        page: 3,
                        hasMore: true,
                      }
                    : {
                        items: [
                          item("CLOUD-2", "云效记录 2"),
                          item("CLOUD-3", "云效记录 3"),
                        ],
                        page: 4,
                        hasMore: false,
                      };
            if (q.deferHistory)
              await new Promise((resolve) => q.pending.push(resolve));
            if (failure) throw failure;
            return clone(result);
          }
          throw Error(`Unexpected IPC ${cmd}`);
        },
      };
    },
    { draft, metadata },
  );
  await page.goto(server.baseUrl);
  await page.locator("#field-title input").waitFor();
  const root = page.locator(".history-view");
  await page.getByRole("button", { name: "历史提交", exact: true }).click();
  await root
    .getByRole("heading", { name: "本机提交记录", exact: true })
    .waitFor();
  assert.equal(await root.getByRole("tabpanel").count(), 1);
  assert.equal(await root.locator(".submission").count(), 20);
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((v) => v.cmd === "list_remote_history").length,
    ),
    0,
  );
  assert.equal(await root.locator("select").count(), 0);
  const tabs = await root.getByRole("tab").evaluateAll((nodes) =>
    nodes.map((node) => ({
      x: node.getBoundingClientRect().x,
      y: node.getBoundingClientRect().y,
    })),
  );
  assert.equal(tabs[0].y, tabs[1].y);
  assert.ok(tabs[1].x > tabs[0].x);
  const stickyTabs = root.locator(".history-tabs");
  const initialTabTop = await stickyTabs.evaluate(
    (node) => node.getBoundingClientRect().top,
  );
  await page.evaluate(() => window.scrollTo(0, 600));
  await page.waitForFunction(
    () =>
      Math.abs(
        document.querySelector(".history-tabs").getBoundingClientRect().top,
      ) < 1,
  );
  assert.equal(
    await stickyTabs.evaluate((node) => getComputedStyle(node).position),
    "sticky",
  );
  assert.ok(initialTabTop > 0);
  await page.screenshot({
    path: path.join(output, "sticky-tabs-scrolled.png"),
  });
  await page.evaluate(() => window.scrollTo(0, 0));
  const firstLocalHeight = await root
    .locator(".submission")
    .first()
    .evaluate((node) => node.getBoundingClientRect().height);
  assert.ok(firstLocalHeight < 150, `local row height: ${firstLocalHeight}`);
  scenarios.push(
    "horizontal tabs stay fixed while scrolling; local rows are compact; one active panel; no cloud calls on local entry",
  );
  await root.getByRole("button", { name: "下一页", exact: true }).click();
  assert.equal(await root.locator(".submission").count(), 10);
  assert.equal(
    await root
      .locator(".submission")
      .filter({ hasText: "本机问题 31" })
      .count(),
    0,
  );
  await root.getByRole("button", { name: "上一页", exact: true }).click();
  scenarios.push(
    "local pagination caps rendered cards at 20 and excludes other projects",
  );
  const choose = async (label, name) => {
    await root
      .getByRole("combobox", { name: `筛选${label}`, exact: true })
      .click();
    await root.getByRole("option", { name, exact: true }).click();
    assert.equal(await root.getByRole("listbox").count(), 1);
    await root.getByRole("heading", { level: 2 }).click();
  };
  for (const [label, name] of [
    ["问题类型", "UI"],
    ["云效 Bug 类型", "缺陷"],
    ["负责人", "郭志强"],
    ["迭代", "当前迭代"],
    ["验证者", "验证者"],
    ["优先级", "中"],
    ["严重程度", "3—一般"],
    ["参与者", "参与人"],
    ["抄送", "抄送人"],
  ])
    await choose(label, name);
  assert.equal(await root.locator(".submission").count(), 15);
  assert.ok((await root.innerText()).includes("共 15 条"));
  scenarios.push(
    "nine intersecting local filters and multi-select menus remain open",
  );
  await root.getByRole("textbox", { name: "搜索历史" }).fill("BUG-2");
  assert.equal(await root.locator(".submission").count(), 6);
  await root.getByRole("button", { name: "清空条件", exact: true }).click();
  assert.equal(await root.locator(".submission").count(), 20);
  const localCard = root
    .locator(".submission")
    .filter({ hasText: "本机问题 30" });
  await localCard
    .getByRole("button", { name: "复制链接", exact: true })
    .click();
  await localCard
    .getByRole("button", { name: "复制修复任务输入", exact: true })
    .click();
  assert.equal(
    await localCard
      .getByRole("button", { name: "核对并恢复", exact: true })
      .count(),
    1,
  );
  assert.equal(
    await root
      .locator(".submission")
      .filter({ hasText: "本机问题 29" })
      .getByRole("button", { name: "核对已有缺陷", exact: true })
      .count(),
    1,
  );
  scenarios.push(
    "local keyword search, clear, and existing receipt/recovery actions preserved",
  );
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
  await root.getByText("云效记录 2", { exact: true }).waitFor();
  assert.equal(await root.locator(".submission").count(), 2);
  const firstRemoteHeight = await root
    .locator(".submission")
    .first()
    .evaluate((node) => node.getBoundingClientRect().height);
  assert.ok(firstRemoteHeight < 90, `remote row height: ${firstRemoteHeight}`);
  await root.getByRole("button", { name: "加载更多", exact: true }).click();
  await root.getByText("云效记录 3", { exact: true }).waitFor();
  assert.equal(await root.locator(".submission").count(), 3);
  const queries = await page.evaluate(() =>
    window.__qa.calls
      .filter((v) => v.cmd === "list_remote_history")
      .map((v) => v.args),
  );
  assert.equal(queries[0].projectId, "test-project");
  assert.equal(queries[1].page, 4);
  assert.deepEqual(queries[1].filters, queries[0].filters);
  scenarios.push(
    "cloud requests scoped to main project; consumed cursor resumes at 4; duplicate IDs removed",
  );
  for (const [label, name] of [
    ["问题类型", "UI"],
    ["云效 Bug 类型", "缺陷"],
    ["负责人", "郭志强"],
    ["迭代", "当前迭代"],
    ["验证者", "验证者"],
    ["优先级", "中"],
    ["严重程度", "3—一般"],
    ["参与者", "参与人"],
    ["抄送", "抄送人"],
  ])
    await choose(label, name);
  await root.getByRole("textbox", { name: "搜索历史" }).fill("图片");
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((v) => v.cmd === "list_remote_history").length,
    ),
    2,
  );
  assert.equal(await root.locator(".submission").count(), 0);
  await root.getByRole("button", { name: "筛选", exact: true }).click();
  await root.getByText("图片筛选命中", { exact: true }).waitFor();
  const filtered = await page.evaluate(
    () =>
      window.__qa.calls.filter((v) => v.cmd === "list_remote_history").at(-1)
        .args,
  );
  assert.equal(filtered.page, 1);
  assert.equal(filtered.filters.keyword, "图片");
  for (const key of [
    "kind",
    "workitemTypeId",
    "assignedTo",
    "sprint",
    "verifier",
    "priority",
    "severity",
    "participants",
    "trackers",
  ])
    assert.equal(filtered.filters[key].length, 1);
  assert.equal(
    await page.evaluate(
      () => window.__qa.calls.filter((v) => v.cmd === "select_project").length,
    ),
    0,
  );
  scenarios.push(
    "cloud Apply sends all nine conditions; edits hide stale results without network or main-project changes",
  );
  await root.getByRole("tab", { name: "本机提交记录", exact: true }).click();
  assert.equal(await root.locator(".submission").count(), 20);
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
  assert.equal(await root.locator(".submission").count(), 1);
  scenarios.push(
    "tabs preserve their independent filter selections and cached results",
  );
  await root.getByRole("button", { name: "清空条件", exact: true }).click();
  await root.getByText("云效记录 2", { exact: true }).waitFor();
  await page.evaluate(() => (window.__qa.rejectHistory = "模拟云效权限不足"));
  await root.getByRole("button", { name: "刷新", exact: true }).click();
  await root
    .getByRole("alert")
    .filter({ hasText: "模拟云效权限不足" })
    .waitFor();
  assert.equal(
    await root
      .getByText("当前账号在此项目下没有符合筛选条件的历史 Bug。", {
        exact: true,
      })
      .count(),
    0,
  );
  await page.evaluate(() => {
    window.__qa.rejectHistory = "";
    window.__qa.sparseHistory = true;
  });
  await root.getByRole("button", { name: "刷新", exact: true }).click();
  await root
    .getByText("本批次没有匹配记录，可继续加载后续记录。", { exact: true })
    .waitFor();
  await root.getByRole("button", { name: "加载更多", exact: true }).click();
  await root.getByText("云效记录 3", { exact: true }).waitFor();
  scenarios.push(
    "permission errors differ from empty results; sparse batches retain Load More",
  );
  await page.evaluate(() => {
    window.__qa.sparseHistory = false;
    window.__qa.deferHistory = true;
  });
  await root.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(() => window.__qa.pending.length === 1);
  await root.getByRole("tab", { name: "本机提交记录", exact: true }).click();
  await page.evaluate(() => (window.__qa.deferHistory = false));
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
  await root.getByText("云效记录 2", { exact: true }).waitFor();
  await page.evaluate(() =>
    window.__qa.pending.splice(0).forEach((resolve) => resolve()),
  );
  await page.waitForFunction(
    () => !document.querySelector('[role="tabpanel"][aria-busy="true"]'),
  );
  assert.equal(await root.locator(".submission").count(), 2);
  scenarios.push(
    "tab changes cancel pending results; returning does not lose the cloud refresh",
  );
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).focus();
  await page.keyboard.press("ArrowLeft");
  assert.equal(
    await root
      .getByRole("tab", { name: "本机提交记录", exact: true })
      .getAttribute("aria-selected"),
    "true",
  );
  await root.getByRole("tab").evaluateAll(async (nodes) => {
    await Promise.all(
      nodes
        .flatMap((node) => node.getAnimations())
        .map((animation) => animation.finished.catch(() => {})),
    );
  });
  await page.screenshot({
    path: path.join(output, "local-tabs-filters.png"),
    fullPage: true,
  });
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
  await page.evaluate(() => (window.__qa.deferHistory = true));
  await root.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(() => window.__qa.pending.length === 1);
  await page.getByRole("button", { name: "新建提交", exact: true }).click();
  await page.getByRole("combobox", { name: "当前项目", exact: true }).click();
  await page.getByRole("option", { name: "另一个项目", exact: true }).click();
  await page.waitForFunction(
    () => window.__qa.profile.projectId === "other-project",
  );
  await page.waitForFunction(() => {
    const project = document.querySelector(
      '[role="combobox"][aria-label="当前项目"]',
    );
    return project?.textContent.includes("另一个项目") && !project.disabled;
  });
  await page.evaluate(() => {
    window.__qa.deferHistory = false;
    window.__qa.pending.splice(0).forEach((resolve) => resolve());
  });
  await page.getByRole("button", { name: "历史提交", exact: true }).click();
  await root
    .getByRole("heading", { name: "本机提交记录", exact: true })
    .waitFor();
  assert.equal(await root.locator(".submission").count(), 5);
  await root.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
  await root.getByText("另一个项目 云效记录 1", { exact: true }).waitFor();
  assert.equal(
    await root.getByText("模拟验收项目 云效记录 1", { exact: true }).count(),
    0,
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((v) => v.cmd === "list_remote_history").at(-1)
          .args.projectId,
    ),
    "other-project",
  );
  scenarios.push(
    "keyboard tab navigation and main-page project switch discard old requests and reset filters",
  );
  await root.getByRole("tab").evaluateAll(async (nodes) => {
    await Promise.all(
      nodes
        .flatMap((node) => node.getAnimations())
        .map((animation) => animation.finished.catch(() => {})),
    );
  });
  await page.screenshot({
    path: path.join(output, "cloud-tabs-filters.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 980, height: 1000 });
  assert.ok(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  );
  scenarios.push("980px narrow layout has no horizontal overflow");
  assert.deepEqual(errors, []);
  await server.verifySource();
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "actual React with mock IPC; Rust request/row matching tested separately",
        scenarios,
        calls: await page.evaluate(() => window.__qa.calls),
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`${scenarios.length} history scenarios passed`);
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

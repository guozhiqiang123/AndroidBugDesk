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
        projectBugStatus: { id: "open", name: "待处理" },
        rejectStatusUpdate: false,
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
              adbPath: null,
              developerOptions: { debugEnabled: false },
              shortcutSettings: { version: 1, overrides: {} },
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
          if (
            cmd === "copy_receipt" ||
            cmd === "open_receipt" ||
            cmd === "copy_history_link" ||
            cmd === "open_history_item"
          )
            return;
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
          if (cmd === "project_bug_facets")
            return {
              statuses: [{ id: "open", name: "待处理" }],
              members: [clone(metadata.members[0])],
              warnings: [],
            };
          if (cmd === "project_bug_workflow")
            return [
              { id: "open", name: "待处理" },
              { id: "doing", name: "处理中" },
              { id: "fixed", name: "已修复" },
              { id: "closed", name: "已关闭" },
            ];
          if (cmd === "update_project_bug_status") {
            if (q.rejectStatusUpdate) throw Error("云效拒绝当前状态流转");
            const statuses = {
              open: "待处理",
              doing: "处理中",
              fixed: "已修复",
              closed: "已关闭",
            };
            if (!statuses[args.statusId]) throw Error("目标状态不在工作流中");
            q.projectBugStatus = {
              id: args.statusId,
              name: statuses[args.statusId],
            };
            return clone(q.projectBugStatus);
          }
          if (cmd === "list_project_bugs")
            return {
              items: [
                {
                  id: "bug-detail-1",
                  identifier: "SKII-614",
                  title: "属于清理的部分功能无法使用（见录屏）",
                  status: clone(q.projectBugStatus),
                  assignedTo: clone(metadata.members[0]),
                  creator: { id: "creator", name: "测试同学" },
                  labels: [{ id: "android", name: "Android" }],
                  createdAt: "2026-10-16T08:00:00Z",
                  updatedAt: "2026-10-16T09:00:00Z",
                },
              ],
              page: 1,
              perPage: 50,
              total: 1,
              totalPages: 1,
              hasMore: false,
            };
          if (cmd === "project_bug_detail")
            return {
              id: "bug-detail-1",
              identifier: "SKII-614",
              title: "属于清理的部分功能无法使用（见录屏）",
              status: clone(q.projectBugStatus),
              assignedTo: clone(metadata.members[0]),
              creator: { id: "creator", name: "测试同学" },
              modifier: { id: "modifier", name: "修改人" },
              verifier: { id: "verifier", name: "验证者" },
              participants: [{ id: "participant", name: "参与人" }],
              trackers: [{ id: "tracker", name: "抄送人" }],
              sprint: { id: "sprint", name: "当前迭代" },
              labels: [{ id: "android", name: "Android" }],
              createdAt: "2026-10-16T08:00:00Z",
              updatedAt: "2026-10-16T09:00:00Z",
              statusUpdatedAt: "2026-10-16T08:30:00Z",
              archived: false,
              description:
                '<p>完整富文本详情</p><a href="https://evil.example">外部链接</a><script>window.__unsafeRan=true</script><img src="https://devops.aliyun.com/projex/api/workitem/file/url?fileIdentifier=inline-image" alt="正文图片">',
              format: "richtext",
              customFields: [
                {
                  id: "severity",
                  name: "严重程度",
                  format: "list",
                  values: ["2-严重"],
                },
              ],
              inlineFiles: [
                {
                  fileId: "inline-image",
                  name: "image.png",
                  suffix: ".png",
                  size: 68,
                },
              ],
              attachments: [
                {
                  id: "text-attachment",
                  fileId: "file-text",
                  name: "diagnostic.txt",
                  suffix: ".txt",
                  size: 32,
                  creator: { id: "creator", name: "测试同学" },
                  createdAt: "2026-10-16T08:11:00Z",
                },
                {
                  id: "system-attachment",
                  fileId: "file-system",
                  name: "recording.mp4",
                  suffix: ".mp4",
                  size: 1024,
                  creator: { id: "creator", name: "测试同学" },
                  createdAt: "2026-10-16T08:12:00Z",
                },
              ],
              comments: [
                {
                  id: "comment-1",
                  parentId: "",
                  author: { id: "developer", name: "开发同学" },
                  content: "<p>已确认 <strong>可以复现</strong></p>",
                  format: "richtext",
                  createdAt: "2026-10-16T09:10:00Z",
                  updatedAt: "2026-10-16T09:10:00Z",
                  pinned: true,
                },
              ],
              activities: [
                {
                  id: "activity-1",
                  eventType: "workitem.transitioned",
                  actionType: "update",
                  actor: { id: "developer", name: "开发同学" },
                  propertyName: "状态",
                  oldValues: ["待处理"],
                  newValues: ["处理中"],
                  relatedResourceId: "",
                  relatedResourceType: "",
                  occurredAt: "2026-10-16T09:20:00Z",
                },
              ],
              sectionWarnings: [],
            };
          if (cmd === "preview_project_bug_attachment") {
            if (args.attachmentId === "text-attachment")
              return {
                kind: "text",
                name: "diagnostic.txt",
                mime: "text/plain",
                data: "mock adb diagnostic output",
              };
            return {
              kind: "system",
              name: "recording.mp4",
              mime: "video/mp4",
              data: "",
            };
          }
          if (cmd === "preview_project_bug_inline_file") {
            if (args.fileId !== "inline-image")
              throw Error(`Unexpected inline file ${args.fileId}`);
            return {
              kind: "image",
              name: "image.png",
              mime: "image/png",
              data: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0E8AAAAASUVORK5CYII=",
            };
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
  const historyScroll = root.locator(".history-scroll");
  const initialTabTop = await stickyTabs.evaluate(
    (node) => node.getBoundingClientRect().top,
  );
  await historyScroll.evaluate((node) => node.scrollTo(0, 600));
  await page.waitForFunction(
    () => document.querySelector(".history-scroll").scrollTop > 0,
  );
  assert.equal(
    await stickyTabs.evaluate((node) => node.getBoundingClientRect().top),
    initialTabTop,
  );
  assert.ok(initialTabTop > 0);
  assert.equal(await page.evaluate(() => window.scrollY), 0);
  await page.screenshot({
    path: path.join(output, "sticky-tabs-scrolled.png"),
  });
  await historyScroll.evaluate((node) => node.scrollTo(0, 0));
  const firstLocalHeight = await root
    .locator(".submission")
    .first()
    .evaluate((node) => node.getBoundingClientRect().height);
  assert.ok(firstLocalHeight < 150, `local row height: ${firstLocalHeight}`);
  scenarios.push(
    "horizontal tabs stay outside the scrolling content; local rows are compact; one active panel; no cloud calls on local entry",
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
  const remoteCard = root
    .locator(".submission")
    .filter({ hasText: "云效记录 1" });
  await remoteCard
    .getByRole("button", { name: "打开云效", exact: true })
    .click();
  await remoteCard
    .getByRole("button", { name: "复制链接", exact: true })
    .click();
  assert.equal(
    await remoteCard
      .getByRole("button", { name: "复制修复任务输入", exact: true })
      .count(),
    0,
  );
  assert.deepEqual(
    await page.evaluate(() =>
      window.__qa.calls
        .filter((call) =>
          ["open_history_item", "copy_history_link"].includes(call.cmd),
        )
        .map((call) => ({ cmd: call.cmd, args: call.args })),
    ),
    [
      { cmd: "open_history_item", args: { identifier: "CLOUD-1" } },
      { cmd: "copy_history_link", args: { identifier: "CLOUD-1" } },
    ],
  );
  scenarios.push(
    "cloud rows expose safe open and copy-link actions but omit repair input without local evidence",
  );
  const firstRemoteHeight = await root
    .locator(".submission")
    .first()
    .evaluate((node) => node.getBoundingClientRect().height);
  assert.ok(firstRemoteHeight < 130, `remote row height: ${firstRemoteHeight}`);
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
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.getByRole("button", { name: "项目 Bug", exact: true }).click();
  const bugs = page.locator(".project-bugs-view");
  await bugs.getByText("属于清理的部分功能无法使用（见录屏）").waitFor();
  const filterFields = bugs.locator(".project-bug-filters > label");
  assert.equal(
    await filterFields.filter({ hasText: "标签" }).count(),
    0,
    "project Bug filters should no longer show labels",
  );
  const sortBox = await filterFields.filter({ hasText: "排序" }).boundingBox();
  const createdFromBox = await filterFields
    .filter({ hasText: "创建日期从" })
    .boundingBox();
  assert(sortBox && createdFromBox, "sort and date filters should be visible");
  assert(
    Math.abs(sortBox.y - createdFromBox.y) < 2 && sortBox.x < createdFromBox.x,
    "sort should occupy the former label slot before the date filters",
  );
  const statusSelect = bugs.getByRole("combobox", {
    name: "修改 SKII-614 状态",
    exact: true,
  });
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "project_bug_workflow")
          .length,
    ),
    0,
    "workflows should be loaded on demand",
  );
  assert(
    (await statusSelect.getAttribute("class"))?.includes(
      "custom-select-trigger",
    ),
  );
  assert(
    (await statusSelect.locator("xpath=..").getAttribute("class"))?.includes(
      "status-tone-todo",
    ),
  );
  await statusSelect.click();
  await page.getByRole("option", { name: "已修复", exact: true }).waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "project_bug_workflow")
          .length,
    ),
    1,
  );
  await page.getByRole("option", { name: "待处理", exact: true }).click();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter(
          (call) => call.cmd === "update_project_bug_status",
        ).length,
    ),
    0,
    "reselecting the current status should not write",
  );
  await statusSelect.click();
  await page.getByRole("option", { name: "已修复", exact: true }).click();
  await page
    .getByText("SKII-614 已更新为「已修复」", { exact: true })
    .waitFor();
  await statusSelect.getByText("已修复", { exact: true }).waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter(
          (call) =>
            call.cmd === "update_project_bug_status" &&
            call.args.id === "bug-detail-1" &&
            call.args.statusId === "fixed",
        ).length,
    ),
    1,
  );
  assert(
    (await statusSelect.locator("xpath=..").getAttribute("class"))?.includes(
      "status-tone-success",
    ),
  );
  await page.screenshot({
    path: path.join(output, "project-bug-status-colors.png"),
    fullPage: true,
  });
  await statusSelect.click();
  await page.getByRole("option", { name: "待处理", exact: true }).waitFor();
  await page.keyboard.press("Escape");
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "project_bug_workflow")
          .length,
    ),
    1,
    "the current page should reuse a loaded workflow",
  );
  await page.evaluate(() => {
    window.__qa.rejectStatusUpdate = true;
  });
  await statusSelect.click();
  await page.getByRole("option", { name: "已关闭", exact: true }).click();
  await bugs.getByText(/状态操作失败：.*云效拒绝当前状态流转/).waitFor();
  await statusSelect.getByText("已修复", { exact: true }).waitFor();
  await page.evaluate(() => {
    window.__qa.rejectStatusUpdate = false;
  });
  scenarios.push(
    "project Bug statuses use semantic colors and an on-demand workflow dropdown with success and rollback states",
  );
  await bugs
    .getByRole("button", {
      name: "属于清理的部分功能无法使用（见录屏）",
      exact: true,
    })
    .click();
  const detailDialog = page.getByRole("dialog", { name: "SKII-614 · 详情" });
  await detailDialog.getByText("完整富文本详情", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__unsafeRan), undefined);
  assert.equal(
    await detailDialog
      .getByText("外部链接", { exact: true })
      .getAttribute("href"),
    null,
  );
  const detailImage = detailDialog.locator(
    '.project-bug-richtext img[src^="data:image/png"]',
  );
  await detailImage.waitFor({ state: "attached" });
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter(
          (call) => call.cmd === "preview_project_bug_inline_file",
        ).length,
    ),
    1,
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.find(
          (call) => call.cmd === "preview_project_bug_inline_file",
        ).args.fileId,
    ),
    "inline-image",
  );
  await detailImage.click();
  const imagePreview = detailDialog.getByRole("dialog", {
    name: "图片预览：正文图片",
  });
  await imagePreview.locator('img[src^="data:image/png"]').waitFor();
  await page.screenshot({
    path: path.join(output, "project-bug-image-preview.png"),
    fullPage: true,
  });
  await imagePreview
    .getByRole("button", { name: "关闭图片预览", exact: true })
    .click();
  assert.equal(await imagePreview.count(), 0);
  await detailImage.click();
  await imagePreview.waitFor();
  await page.keyboard.press("Escape");
  assert.equal(await imagePreview.count(), 0);
  await detailDialog.waitFor({ state: "visible" });
  assert.equal(
    await detailDialog.getByText("严重程度", { exact: true }).count(),
    1,
  );
  assert.equal(
    await detailDialog.getByText("已确认 可以复现", { exact: true }).count(),
    1,
  );
  assert.equal(
    await detailDialog.getByText("已修复", { exact: true }).count(),
    1,
  );
  await detailDialog
    .locator(".project-bug-attachment-row")
    .filter({ hasText: "diagnostic.txt" })
    .getByRole("button", { name: "预览", exact: true })
    .click();
  await detailDialog
    .getByText("mock adb diagnostic output", { exact: true })
    .waitFor();
  await detailDialog
    .getByRole("button", { name: "关闭附件预览", exact: true })
    .click();
  await detailDialog
    .locator(".project-bug-attachment-row")
    .filter({ hasText: "recording.mp4" })
    .getByRole("button", { name: "预览", exact: true })
    .click();
  await detailDialog
    .getByText("已使用系统应用打开 recording.mp4", { exact: true })
    .waitFor();
  await detailDialog.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(
    () =>
      window.__qa.calls.filter((call) => call.cmd === "project_bug_detail")
        .length === 2,
  );
  await detailDialog
    .getByRole("button", { name: "复制链接", exact: true })
    .click();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls
          .filter((call) => call.cmd === "copy_history_link")
          .at(-1).args.identifier,
    ),
    "SKII-614",
  );
  await detailDialog
    .getByRole("button", { name: "在云效中打开", exact: true })
    .click();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls
          .filter((call) => call.cmd === "open_history_item")
          .at(-1).args.identifier,
    ),
    "SKII-614",
  );
  await page.screenshot({
    path: path.join(output, "project-bug-detail.png"),
    fullPage: true,
  });
  await detailDialog
    .getByRole("button", { name: "关闭SKII-614 · 详情", exact: true })
    .click();
  scenarios.push(
    "project Bug detail stays in-app with sanitized rich text, safe inline images, fields, comments, activities, attachment previews, refresh and cloud fallback",
  );
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

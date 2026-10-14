// Run with PLAYWRIGHT_MODULE pointing at an installed Playwright index.mjs.
// All IPC and devices are mocks. The temporary Vite server is stopped before exit.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  startBrowserCheck,
  recordBrowserFailure,
} from "./browser-check-support.mjs";
import assert from "node:assert/strict";
const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const output = path.resolve(process.env.QA_OUTPUT || "artifacts/gzq49/browser");
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
  metadata.fields.push(
    ...["participants", "trackers"].map((id) => ({
      id,
      name: id === "participants" ? "参与者" : "抄送",
      format: "multiUser",
      type: "Role",
      required: false,
      options: [],
    })),
    {
      id: "labels",
      name: "标签",
      format: "tag",
      type: "SystemCustomField",
      required: false,
      options: [{ id: "label-1", name: "标签一" }],
    },
    {
      id: "plannedStartDate",
      name: "计划开始日期",
      format: "date",
      type: "SystemCustomField",
      required: false,
      options: [],
    },
    {
      id: "notFixReason",
      name: "不修复理由",
      format: "string",
      type: "SystemCustomField",
      required: false,
      options: [],
    },
    {
      id: "estimatedTime",
      name: "预计工时",
      format: "int",
      type: "SystemCustomField",
      required: false,
      options: [],
    },
    {
      id: "legacyHidden",
      name: "历史隐藏字段",
      format: "string",
      type: "SystemCustomField",
      required: false,
      showWhenCreate: false,
      options: [],
    },
  );
  fixture.customFields.labels = ["stale-label"];
  fixture.customFields.plannedStartDate = "invalid-old-date";
  fixture.customFields.notFixReason = "stale-reason";
  fixture.customFields.legacyHidden = "旧值不应显示";
  if (process.env.QA_SCOPE === "input-hints") {
    for (const [id, name, format, required, value] of [
      ["hintText", "必填说明", "text", true, "模拟说明"],
      ["hintInt", "整数配置", "int", false, ""],
      ["hintFloat", "数值配置", "float", false, ""],
      ["hintTime", "时间配置", "dateTime", false, ""],
    ]) {
      metadata.fields.push({
        id,
        name,
        format,
        required,
        type: "CustomField",
        options: [],
      });
      fixture.customFields[id] = value;
    }
  }
  await page.addInitScript(
    ({ fixture, metadata, inputHints, postCapture, startupUpdate }) => {
      const clone = (v) => structuredClone(v);
      fixture.sessionId = postCapture
        ? null
        : "00000000-0000-4000-8000-000000000009";
      const q = (window.__qa = {
        authorized: false,
        user: null,
        profile: {
          ...metadata.profile,
          name: "请选择项目",
          organizationId: "unconfigured",
          projectId: "unconfigured",
        },
        metadata,
        drafts: [fixture],
        draftScopes: {
          [fixture.id]: {
            organizationId: "test-org",
            projectId: "test-project",
          },
        },
        projectSettings: {
          entries: [
            {
              projectId: "test-project",
              enabled: true,
              packageName: "com.example.mock",
            },
          ],
        },
        developerOptions: { debugEnabled: false },
        settingsByOrg: {
          "test-org": {
            entries: [
              {
                projectId: "test-project",
                enabled: true,
                packageName: "com.example.mock",
              },
              {
                projectId: "second-project",
                enabled: false,
                packageName: "com.second.saved",
              },
              {
                projectId: "long-project",
                enabled: false,
                packageName: "",
              },
              {
                projectId: "another-project",
                enabled: false,
                packageName: "",
              },
            ],
          },
          "second-org": {
            entries: [
              { projectId: "second-project", enabled: true, packageName: "" },
            ],
          },
        },
        failSettingsSave: false,
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
      });
      if (inputHints || postCapture) q.captures = [];
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
          q.calls.push({ cmd, args: clone(args) });
          if (cmd === "bootstrap")
            return clone({
              ...q,
              adbPath: "/mock/adb",
              submissions: [],
              sessions: [],
            });
          if (cmd === "record_renderer_event") return;
          if (cmd === "set_developer_options") {
            q.developerOptions = clone(args.options);
            return clone(q.developerOptions);
          }
          if (cmd === "login") {
            if (q.failLogin)
              return { status: "invalid", user: null, message: "模拟令牌无效" };
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
            return [
              { id: "test-org", name: "模拟组织" },
              { id: "second-org", name: "第二组织" },
            ];
          if (cmd === "list_projects")
            return args.organizationId === "second-org"
              ? [{ id: "second-project", name: "第二项目" }]
              : [
                  { id: "test-project", name: "模拟项目" },
                  { id: "second-project", name: "第二项目" },
                  {
                    id: "long-project",
                    name: "LongProjectWithoutSpaces_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789",
                  },
                  { id: "another-project", name: "第四项目" },
                ];
          if (cmd === "load_project_settings")
            return clone(
              q.settingsByOrg[args.organizationId] ?? { entries: [] },
            );
          if (cmd === "save_project_settings") {
            if (q.failSettingsSave) throw "模拟设置保存失败";
            q.settingsByOrg[args.organizationId] = clone(args.settings);
            return clone(q.settingsByOrg[args.organizationId]);
          }
          if (cmd === "check_for_updates") {
            const checkCount = q.calls.filter(
              (call) => call.cmd === "check_for_updates",
            ).length;
            const updateAvailable = startupUpdate || checkCount > 1;
            return {
              ok: true,
              currentVersion: "0.1.0",
              latestVersion: updateAvailable ? "0.2.0" : "0.1.0",
              hasUpdate: updateAvailable,
              releaseUrl: `https://github.com/guozhiqiang123/AndroidBugDesk/releases/tag/v${updateAvailable ? "0.2.0" : "0.1.0"}`,
              publishedAt: "2026-10-12T00:00:00Z",
              releasesChecked: 2,
              checkedAtEpochMs: 1791734400000,
              assetName: updateAvailable
                ? "Android-Bug-Desk-0.2.0-macos-arm64.dmg"
                : "Android-Bug-Desk-0.1.0-macos-arm64.dmg",
              assetSizeBytes: 10485760,
              canAutoUpdate: updateAvailable,
              message: updateAvailable
                ? "发现新版本 v0.2.0，可直接下载安装。"
                : "当前版本 v0.1.0 已是最新正式版本。",
              error: null,
            };
          }
          if (cmd === "install_update")
            return {
              ok: false,
              message: "更新失败。",
              filePath: null,
              error: "模拟下载安装失败",
            };
          if (cmd === "open_release_page") return;
          if (cmd === "select_project") {
            if (q.failProject) throw "模拟项目切换失败";
            q.profile = {
              ...clone(q.profile),
              organizationId: args.organizationId,
              projectId: args.projectId,
              name: "模拟项目",
            };
            return clone(q.profile);
          }
          if (cmd === "choose_adb") {
            if (q.failAdb) throw "模拟 ADB 路径选择失败";
            return "/mock/adb";
          }
          if (cmd === "list_devices") {
            if (q.failDeviceList) throw "模拟设备刷新失败";
            if (q.holdDeviceList)
              await new Promise((resolve) => {
                window.__releaseDeviceList = resolve;
              });
            return clone(
              q.devices ?? [
                {
                  serial: "mock-device",
                  state: "device",
                  label: "Pixel Mock (mock-device)",
                },
              ],
            );
          }
          if (cmd === "inspect_device") {
            if (q.failDevice) throw "模拟设备读取失败";
            const snapshot = {
              bugPage: q.missingPage ? "" : "com.example.mock.MainActivity",
              device: {
                model: "Pixel Mock",
                androidVersion: "14",
                totalMemKb: "8192",
                availableMemKb: "4096",
                resolution: "1080x2400",
                density: "420",
              },
              app: {
                packageName: "com.example.mock",
                versionName: "1.2.3",
                versionCode: "123",
              },
              sources: {
                bugPage: {
                  status: q.missingPage ? "failed" : "collected",
                  at: "2026-10-09T00:00:00Z",
                  detail: "ADB mock page",
                },
                "app.packageName": {
                  status: "collected",
                  at: "2026-10-09T00:00:00Z",
                  detail: "ADB mock",
                },
                "device.model": {
                  status: "collected",
                  at: "2026-10-09T00:00:00Z",
                  detail: "ADB mock",
                },
              },
              errors: [],
              details: { foregroundActivity: "com.example.mock/.MainActivity" },
              deviceTime: "2026-10-09T00:00:00Z",
              capturedAt: "2026-10-09T00:00:00Z",
            };
            Object.assign(draft().app, snapshot.app);
            Object.assign(draft().device, snapshot.device);
            const keepExisting =
              draft().sources.bugPage?.status === "manual" ||
              (!snapshot.bugPage && !!draft().bugPage);
            if (!keepExisting) draft().bugPage = snapshot.bugPage;
            const priorPageSource = draft().sources.bugPage;
            draft().sources = { ...draft().sources, ...snapshot.sources };
            if (keepExisting) draft().sources.bugPage = priorPageSource;
            return clone(snapshot);
          }
          if (cmd === "start_session")
            return {
              id: "00000000-0000-4000-8000-000000000010",
              serial: args.serial,
              packageName: args.packageName,
              startedAt: "2026-10-09T00:00:00Z",
              stoppedAt: null,
              snapshot: {},
              logFiles: [],
              gaps: [],
            };
          if (cmd === "stop_session") return;
          if (cmd === "load_metadata") return clone(q.metadata);
          if (cmd === "new_draft") {
            if (!q.drafts.length)
              q.drafts.push({
                ...clone(fixture),
                id: crypto.randomUUID(),
                sessionId: null,
              });
            return clone(draft());
          }
          if (cmd === "load_selection_memory")
            return clone({
              workitemTypeId: fixture.workitemTypeId,
              assignedTo: fixture.assignedTo,
              versions: fixture.versions,
              customFields: {
                ...fixture.customFields,
                participants: [metadata.members[0].id],
                trackers: [metadata.members[0].id],
              },
            });
          if (cmd === "remember_selections") return;
          if (cmd === "save_draft") {
            const changedPage = args.draft.bugPage !== draft().bugPage;
            q.drafts[0] = {
              ...clone(args.draft),
              attachments: draft().attachments,
            };
            if (changedPage)
              draft().sources.bugPage = {
                status: "manual",
                at: "2026-10-10T00:00:00Z",
                detail: "mock manual page",
              };
            return clone(draft());
          }
          if (cmd === "start_post_capture") {
            if (q.failPostCapture) throw "模拟环境快照启动失败";
            const job = {
              id: crypto.randomUUID(),
              draftId: args.id,
              kind: "post-capture",
              status: "pending",
              serial: args.serial,
              targetPackage: draft().app.packageName,
              issueKind: draft().kind,
              startedAt: "2026-10-10T08:00:00Z",
              finishedAt: null,
              items: Object.fromEntries(
                args.items.map((item) => [
                  item,
                  { status: "pending", attachments: [], commands: [] },
                ]),
              ),
            };
            q.captures.unshift(job);
            q.postCapturePolls = 0;
            return clone(job);
          }
          if (cmd === "post_capture_status") {
            q.postCapturePolls = (q.postCapturePolls ?? 0) + 1;
            const job = q.captures.find(
              (capture) => capture.kind === "post-capture",
            );
            if (
              job?.status === "pending" &&
              job.items.environment &&
              q.postCapturePolls >= 2
            ) {
              job.status = "complete";
              job.finishedAt = "2026-10-10T08:00:01Z";
              for (const item of Object.keys(job.items)) {
                job.items[item] = {
                  status: "complete",
                  attachments: ["00000000-0000-4000-8000-000000000001"],
                  commands: [],
                };
              }
              if (job.items.environment) {
                draft().attachments.push({
                  ...attached(
                    "device-environment-snapshot-mock.txt",
                    "sample",
                    "设备环境快照；项目配置包名 com.example.mock",
                  ),
                  id: "00000000-0000-4000-8000-000000000001",
                  mime: "application/octet-stream",
                });
              }
            }
            return clone({
              draft: draft(),
              jobs: q.captures.filter(
                (capture) => capture.kind === "post-capture",
              ),
            });
          }
          if (cmd === "import_attachment") {
            if (q.cancelImport) throw "已取消";
            if (q.failImport) throw "模拟附件导入失败";
            for (const file of q.importBatch ?? [
              { name: "log-imported.log", role: "log" },
            ])
              draft().attachments.push(
                attached(file.name, file.role, "导入文件，类型自动识别"),
              );
            return clone(draft());
          }
          if (cmd === "collect_software_evidence") {
            const prior = draft().attachments.filter(
              (item) => item.source !== "Android Bug Desk 软件运行诊断",
            );
            prior.push(
              attached(
                "software-evidence-mock.zip",
                "report",
                "Android Bug Desk 软件运行诊断",
              ),
            );
            draft().attachments = prior;
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
          if (
            cmd === "confirm_attachment" ||
            cmd === "save_attachment_coverage"
          ) {
            const a = draft().attachments.find(
              (a) => a.id === args.attachmentId,
            );
            const owner =
              a.role === "report" && a.derivedFrom ? a.derivedFrom : a.id;
            a.coverageStart = args.coverageStart;
            a.coverageEnd = args.coverageEnd;
            if (cmd === "confirm_attachment")
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
          if (cmd === "validate_draft") return clone(q.backendIssues ?? []);
          if (cmd === "preview_attachment") {
            if (q.failPreview)
              throw "无法使用系统应用打开此附件，请设置该格式的默认打开应用后重试";
            return;
          }
          if (cmd === "list_remote_history") {
            if (q.authFailure) throw "[授权失效] HTTP 401";
            return { items: [], page: 1, hasMore: false };
          }
          throw Error("Unexpected IPC " + cmd);
        },
      };
    },
    {
      fixture,
      metadata,
      inputHints: process.env.QA_SCOPE === "input-hints",
      postCapture: ["post-capture", "device-connection-row"].includes(
        process.env.QA_SCOPE,
      ),
      startupUpdate: process.env.QA_SCOPE === "startup-update",
    },
  );
  await page.goto(server.baseUrl);
  await page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  if (process.env.QA_SCOPE === "startup-update") {
    const dialog = page.getByRole("dialog", {
      name: "发现新版本",
      exact: true,
    });
    await dialog.waitFor();
    assert.match(await dialog.innerText(), /v0\.1\.0 → v0\.2\.0/);
    assert.equal(
      await dialog
        .getByRole("button", { name: "立马更新", exact: true })
        .count(),
      1,
    );
    assert.equal(
      await dialog.getByRole("button", { name: "取消", exact: true }).count(),
      1,
    );
    assert.equal(await dialog.locator(".app-dialog-close").count(), 1);
    await dialog.getByRole("button", { name: "取消", exact: true }).click();
    await dialog.waitFor({ state: "detached" });
    assert.equal(
      await page.evaluate(
        () =>
          window.__qa.calls.filter((call) => call.cmd === "install_update")
            .length,
      ),
      0,
    );
    await page.reload();
    await page
      .getByRole("heading", { name: "登录云效", exact: true })
      .waitFor();
    const reloadedDialog = page.getByRole("dialog", {
      name: "发现新版本",
      exact: true,
    });
    await reloadedDialog.waitFor();
    await reloadedDialog
      .getByRole("button", { name: "立马更新", exact: true })
      .click();
    await page
      .getByRole("heading", { name: "更新失败", exact: true })
      .waitFor();
    assert.equal(
      await page.evaluate(
        () =>
          window.__qa.calls.filter((call) => call.cmd === "install_update")
            .length,
      ),
      1,
    );
    break checks;
  }
  await page.getByRole("tab", { name: "填写 PAT", exact: true }).click();
  assert.equal(await page.locator("input[type=password]").inputValue(), "");
  await page.waitForFunction(() =>
    window.__qa.calls.some((call) => call.cmd === "check_for_updates"),
  );
  assert.equal(
    await page.evaluate(
      () => window.__qa.calls.filter((call) => call.cmd === "bootstrap").length,
    ),
    1,
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "check_for_updates")
          .length,
    ),
    1,
  );
  await page.locator("input[type=password]").fill("mock-only");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator("#field-title input").waitFor();
  assert.equal(
    await page.locator(".user-profile strong").innerText(),
    "模拟昵称",
  );
  assert.ok(
    (await page.evaluate(() => window.__qa.calls.map((c) => c.cmd))).includes(
      "select_project",
    ),
    "unconfigured login selects the first real organization/project",
  );
  assert.ok(
    await page.locator('[id="field-title"] input').count(),
    "draft loaded after project selection",
  );
  if (process.env.QA_SCOPE === "device-connection-row") {
    const connection = page.locator(
      "#field-attachments .evidence-device-connection",
    );
    await connection.waitFor();
    const countCalls = (command) =>
      page.evaluate(
        (cmd) => window.__qa.calls.filter((call) => call.cmd === cmd).length,
        command,
      );
    await page.waitForFunction(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "list_devices")
          .length === 1 &&
        document.querySelector(".evidence-device-connection")?.disabled ===
          false,
    );
    assert.equal(await countCalls("inspect_device"), 1);
    const deviceReadNotice = page.locator(".notification-toast.success");
    await deviceReadNotice
      .getByText("已读取当前设备和配置应用信息。", { exact: true })
      .waitFor();
    assert.equal(
      await deviceReadNotice.getByText(/前台页面信息/).count(),
      0,
      "设备读取成功提示不再展示前台页面文案",
    );
    const postCapture = page.getByRole("button", {
      name: "补采证据",
      exact: true,
    });
    assert.equal(await postCapture.isDisabled(), false);
    assert.equal(
      await postCapture.getAttribute("title"),
      "补采结果可在附件列表查看",
    );
    assert.equal(
      await postCapture.locator(".post-capture-info-badge").count(),
      1,
    );
    assert.equal(
      await page.getByText("补采结果可在此查看", { exact: true }).count(),
      0,
      "补采入口说明应收进按钮提示，不再占用按钮下方空间",
    );
    assert.equal(
      await page
        .getByRole("button", {
          name: "重新读取当前前台界面",
          exact: true,
        })
        .count(),
      0,
    );
    assert.equal(await connection.locator(".device-connection-row").count(), 1);
    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.evaluate(() => {
      window.__qa.devices = [];
    });
    await page.getByRole("button", { name: "新建提交", exact: true }).click();
    await page.waitForFunction(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "list_devices")
          .length === 2 &&
        document.querySelector(".evidence-device-connection")?.disabled ===
          false,
    );
    assert.equal(await postCapture.isDisabled(), true);
    assert.match(
      await connection
        .getByRole("combobox", { name: "目标设备", exact: true })
        .innerText(),
      /暂无可选项/,
    );
    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.evaluate(() => {
      window.__qa.failDeviceList = true;
    });
    await page.getByRole("button", { name: "新建提交", exact: true }).click();
    await page.waitForFunction(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "list_devices")
          .length === 3 &&
        document
          .getElementById("field-device.serial")
          ?.textContent.includes("模拟设备刷新失败"),
    );
    assert.equal(await postCapture.isDisabled(), true);
    await page.evaluate(() => {
      window.__qa.failDeviceList = false;
      window.__qa.devices = [
        {
          serial: "mock-device",
          state: "device",
          label: "Pixel Mock (mock-device)",
        },
        { serial: "second-device", state: "device", label: "Second Phone" },
      ];
    });
    const beforeRefresh = await countCalls("list_devices");
    const beforeRead = await countCalls("inspect_device");
    await connection
      .getByRole("button", { name: "刷新设备", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(".evidence-device-connection")?.disabled ===
        false,
    );
    assert.equal(await countCalls("list_devices"), beforeRefresh + 1);
    assert.equal(await countCalls("inspect_device"), beforeRead + 1);
    assert.equal(await postCapture.isDisabled(), false);
    const device = connection.getByRole("combobox", {
      name: "目标设备",
      exact: true,
    });
    await device.click();
    await page
      .getByRole("option", { name: "Second Phone", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(".evidence-device-connection")?.disabled ===
        false,
    );
    assert.equal(await page.getByRole("listbox").count(), 0);
    assert.equal(
      await page.evaluate(
        () =>
          window.__qa.calls
            .filter((call) => call.cmd === "inspect_device")
            .at(-1).args.serial,
      ),
      "second-device",
    );
    for (const size of [
      { width: 980, height: 700 },
      { width: 1280, height: 900 },
      { width: 1440, height: 1000 },
    ]) {
      await page.setViewportSize(size);
      const layout = await connection.evaluate((node) => {
        const selector = node
          .querySelector(".custom-select-trigger")
          .getBoundingClientRect();
        const actions = [
          ...node.querySelectorAll(
            ".device-connection-row > .connection-button-field > button",
          ),
        ].map((item) => item.getBoundingClientRect());
        return {
          actionCount: actions.length,
          aligned: actions.every(
            (action) => Math.abs(selector.top - action.top) < 2,
          ),
          ordered: actions.every(
            (action, index) =>
              action.left >
              (index === 0 ? selector.right : actions[index - 1].right),
          ),
          overflow: document.documentElement.scrollWidth > innerWidth,
        };
      });
      assert.deepEqual(layout, {
        actionCount: 2,
        aligned: true,
        ordered: true,
        overflow: false,
      });
      await connection.screenshot({
        path: path.join(output, `device-connection-row-${size.width}.png`),
      });
    }
    assert.deepEqual(errors, []);
    assert.equal(await countCalls("submit_bug"), 0);
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "Actual React device connection row with mocked IPC; no real USB/cloud",
          scenarios: [
            "standalone foreground reread action is absent",
            "entering the main page refreshes devices once",
            "missing devices clear stale selection and disable post-capture",
            "device refresh failures keep post-capture disabled",
            "connected ADB device enables post-capture",
            "device selector, ADB chooser and refresh action share one row",
            "refresh and device selection retain automatic inspection",
            "single-select menu closes after choosing a device",
            "980/1280/1440 layouts align controls without horizontal overflow",
          ],
        },
        null,
        2,
      ) + "\n",
    );
    console.log("9 device connection row scenarios passed; no real USB/cloud.");
    break checks;
  }
  if (process.env.QA_SCOPE === "post-capture") {
    assert.equal(
      await page
        .getByRole("button", { name: "应用与设备", exact: true })
        .count(),
      0,
    );
    const connection = page.locator(
      "#field-attachments .evidence-device-connection",
    );
    await connection
      .getByRole("button", { name: "刷新设备", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(".evidence-device-connection")?.disabled ===
        false,
    );
    const before = await page.locator(".attachment").count();
    await page
      .getByRole("button", { name: "设备环境快照", exact: true })
      .click();
    await page
      .locator(".notification-toast.success", {
        hasText: "设备环境快照已获取，文本已加入附件",
      })
      .waitFor();
    assert.equal(
      await page
        .locator("#field-environmentSnapshot > small", {
          hasText: "文本已加入附件",
        })
        .count(),
      0,
      "环境快照完成提示不再显示在按钮下方",
    );
    assert.equal(await page.locator(".attachment").count(), before + 1);
    const snapshotCard = page
      .locator(".attachment")
      .filter({ hasText: "device-environment-snapshot-mock.txt" });
    await snapshotCard.waitFor();
    assert.deepEqual(
      await snapshotCard
        .locator(".attachment-capture-item span")
        .allInnerTexts(),
      ["采集项", "设备与应用环境"],
    );
    assert.equal(
      await snapshotCard.locator(".attachment-meta").innerText(),
      "数据文件 · 0.00 MB",
    );
    await snapshotCard
      .getByRole("button", { name: "预览", exact: true })
      .click();
    const preview = await page.evaluate(() =>
      window.__qa.calls.find(
        (call) =>
          call.cmd === "preview_attachment" &&
          call.args.attachmentId === "00000000-0000-4000-8000-000000000001",
      ),
    );
    assert.ok(preview, "环境快照沿用附件的系统预览入口");
    const start = await page.evaluate(() =>
      window.__qa.calls.find((call) => call.cmd === "start_post_capture"),
    );
    assert.deepEqual(start.args.items, ["environment"]);
    assert.equal(start.args.serial, "mock-device");
    assert.equal(
      await page.locator("#field-environmentSnapshot .error-text").count(),
      0,
    );
    await page.evaluate(() => {
      const base = {
        draftId: window.__qa.drafts[0].id,
        kind: "post-capture",
        serial: "mock-device",
        targetPackage: "com.example.mock",
        issueKind: "功能",
        finishedAt: null,
        error: null,
      };
      window.__qa.captures = [
        {
          ...base,
          id: "visible-running",
          status: "pending",
          startedAt: new Date(Date.now() - 3000).toISOString(),
          items: {
            screenshot: {
              status: "complete",
              attachments: ["new-screen"],
              commands: [],
            },
            bugreport: {
              status: "running",
              startedAt: new Date(Date.now() - 3000).toISOString(),
              attachments: [],
              commands: [],
            },
          },
        },
        {
          ...base,
          id: "visible-older",
          status: "complete",
          startedAt: "2026-10-10T07:00:00Z",
          finishedAt: "2026-10-10T07:01:00Z",
          items: {
            logs: {
              status: "complete",
              attachments: ["old-log"],
              commands: [],
            },
            screenshot: {
              status: "complete",
              attachments: ["old-screen"],
              commands: [],
            },
          },
        },
      ];
      window.__qa.postCapturePolls = 0;
    });
    await page.getByRole("button", { name: "补采证据", exact: true }).click();
    const captureDialog = page.getByRole("dialog", {
      name: "补采证据",
      exact: true,
    });
    await captureDialog.waitFor();
    await page.setViewportSize({ width: 1280, height: 640 });
    const closeCapture = captureDialog.getByRole("button", {
      name: "关闭补采",
      exact: true,
    });
    assert.equal((await closeCapture.textContent()).trim(), "");
    assert.equal(await closeCapture.locator("svg").count(), 1);
    const dialogChrome = await captureDialog.evaluate((node) => {
      const style = getComputedStyle(node);
      const scroll = node.querySelector(".app-dialog-scroll");
      const title = node.querySelector(".app-dialog-titlebar");
      const close = node.querySelector(".app-dialog-close");
      return {
        overflow: style.overflow,
        topRightRadius: style.borderTopRightRadius,
        bottomRightRadius: style.borderBottomRightRadius,
        scrollable: scroll.scrollHeight > scroll.clientHeight,
        titleTop: title.getBoundingClientRect().top,
        closeTop: close.getBoundingClientRect().top,
      };
    });
    assert.deepEqual(
      {
        overflow: dialogChrome.overflow,
        topRightRadius: dialogChrome.topRightRadius,
        bottomRightRadius: dialogChrome.bottomRightRadius,
        scrollable: dialogChrome.scrollable,
      },
      {
        overflow: "hidden",
        topRightRadius: "14px",
        bottomRightRadius: "14px",
        scrollable: true,
      },
    );
    await captureDialog
      .locator(".app-dialog-scroll")
      .evaluate((node) => (node.scrollTop = node.scrollHeight));
    const fixedChrome = await captureDialog.evaluate((node) => ({
      titleTop: node
        .querySelector(".app-dialog-titlebar")
        .getBoundingClientRect().top,
      closeTop: node.querySelector(".app-dialog-close").getBoundingClientRect()
        .top,
    }));
    assert.ok(
      Math.abs(fixedChrome.titleTop - dialogChrome.titleTop) < 1 &&
        Math.abs(fixedChrome.closeTop - dialogChrome.closeTop) < 1,
      "补采标题栏和关闭按钮不随正文滚动",
    );
    assert.equal(
      await captureDialog.getByRole("checkbox").count(),
      14,
      "环境快照不在补采选项中重复出现",
    );
    assert.equal(await captureDialog.getByText("设备与应用环境").count(), 0);
    const checked = await captureDialog
      .getByRole("checkbox")
      .evaluateAll((nodes) =>
        nodes
          .filter((node) => node.checked)
          .map((node) => node.parentElement.textContent.trim()),
      );
    assert.deepEqual(checked, ["当前界面层级", "应用与进程状态"]);
    assert.equal(
      await captureDialog.locator("section.post-capture-result").count(),
      1,
      "多次补采合并成一个最新结果区域",
    );
    assert.equal(
      await captureDialog.locator(".post-capture-result-item").count(),
      3,
      "重复项目只显示最新一次",
    );
    await captureDialog
      .getByText(/系统报告已生成 \d+ 秒，通常需要 1–3 分钟/)
      .waitFor();
    await captureDialog.screenshot({
      path: path.join(output, "consolidated-running-bugreport.png"),
    });
    await page.evaluate(() => {
      const job = window.__qa.captures[0];
      job.status = "complete";
      job.finishedAt = new Date().toISOString();
      job.items.bugreport.status = "complete";
      job.items.bugreport.finishedAt = job.finishedAt;
    });
    const captureCompleteNotice = page.locator(".notification-toast.success", {
      hasText: "补采完成",
    });
    await captureCompleteNotice.waitFor();
    await captureDialog.waitFor({ state: "detached" });
    assert.equal(
      await page.getByRole("dialog", { name: "补采证据" }).count(),
      0,
      "全部选中项目采集完成后自动关闭补采弹窗",
    );
    assert.equal(
      await page
        .locator("#field-postCapture > small", { hasText: "补采完成" })
        .count(),
      0,
      "补采完成不再显示在补采按钮下方",
    );
    await page.getByRole("button", { name: "补采证据", exact: true }).click();
    const completedCaptureDialog = page.getByRole("dialog", {
      name: "补采证据",
      exact: true,
    });
    await completedCaptureDialog.waitFor();
    await completedCaptureDialog
      .getByText("最近补采结果（每项仅显示最新一次）", { exact: true })
      .waitFor();
    await completedCaptureDialog
      .getByRole("button", { name: "关闭补采", exact: true })
      .click();
    await page.getByRole("button", { name: "设置", exact: true }).click();
    const packageInput = page.getByRole("textbox", {
      name: "模拟项目 Android 包名",
      exact: true,
    });
    await packageInput.fill("");
    await page.getByRole("button", { name: "保存设置", exact: true }).click();
    await page.getByRole("button", { name: "新建提交", exact: true }).click();
    await page.getByRole("heading", { name: "请先配置当前项目包名" }).waitFor();
    assert.equal(await page.locator("#field-title").count(), 0);
    assert.equal(
      await page.getByRole("button", { name: "导入附件", exact: true }).count(),
      0,
    );
    await page.getByRole("button", { name: "前往设置", exact: true }).click();
    const newDraftCalls = await page.evaluate(
      () => window.__qa.calls.filter((call) => call.cmd === "new_draft").length,
    );
    await page.getByRole("button", { name: "退出登录", exact: true }).click();
    await page
      .getByRole("heading", { name: "登录云效", exact: true })
      .waitFor();
    await page.getByRole("tab", { name: "填写 PAT", exact: true }).click();
    await page.locator("input[type=password]").fill("mock-only");
    await page.getByRole("button", { name: "确定", exact: true }).click();
    await page.getByRole("heading", { name: "请先配置当前项目包名" }).waitFor();
    assert.equal(
      await page.evaluate(
        () =>
          window.__qa.calls.filter((call) => call.cmd === "new_draft").length,
      ),
      newDraftCalls,
      "启动时缺少包名只显示设置门禁，不创建新草稿",
    );
    assert.equal(await page.locator("main > .banner.error").count(), 0);
    await page.getByRole("button", { name: "前往设置", exact: true }).click();
    await packageInput.fill("com.example.restored");
    await page.getByRole("button", { name: "保存设置", exact: true }).click();
    await page.getByRole("button", { name: "新建提交", exact: true }).click();
    await page.locator("#field-title input").waitFor();
    await page.evaluate(() => (window.__qa.cancelImport = true));
    await page.getByRole("button", { name: "导入附件", exact: true }).click();
    await page
      .locator(".notification-toast.warning", { hasText: "已取消导入附件" })
      .waitFor();
    assert.equal(
      await page
        .locator("#field-attachment-import .error-text", { hasText: "已取消" })
        .count(),
      0,
      "取消导入作为右下角提示，不显示在按钮下方",
    );
    await page.evaluate(() => {
      window.__qa.cancelImport = false;
      window.__qa.importBatch = [{ name: "toast-import.png", role: "image" }];
    });
    await page.getByRole("button", { name: "导入附件", exact: true }).click();
    await page
      .locator(".notification-toast.success", { hasText: "附件已导入" })
      .waitFor();
    await page
      .locator(".attachment", { hasText: "toast-import.png" })
      .waitFor();
    await page.locator("#field-attachments").screenshot({
      path: path.join(output, "environment-snapshot-entry.png"),
    });
    assert.deepEqual(errors, []);
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          scenarios: [
            "removed application/device dialog",
            "environment snapshot completion uses the bottom-right notice",
            "one-click environment snapshot attaches previewable text",
            "environment snapshot uses the system attachment preview entry",
            "post-capture recommendations exclude the separate snapshot",
            "repeated jobs collapse to the latest result for each item",
            "running bugreport shows elapsed time and its expected duration",
            "post-capture completion closes the dialog and uses the bottom-right notice",
            "opening completed capture history keeps the dialog available for review",
            "post-capture dialog clips rounded corners and keeps an icon title bar fixed",
            "attachment import success and cancellation use bottom-right notices",
            "missing project package blocks the whole new-submission page",
            "login with a missing package shows the gate without creating a draft",
            "saved package restores the page",
          ],
          mode: "Actual React with mocked IPC; real USB is covered by Rust smoke verification",
        },
        null,
        2,
      ) + "\n",
    );
    break checks;
  }
  if (process.env.QA_SCOPE === "input-hints") {
    const input = (id) =>
      page.locator(`[id="field-${id}"]`).locator("input, textarea");
    const messages = (id) => page.locator(`[id="field-${id}"] > small`);
    const previewButton = page.getByRole("button", {
      name: "进入提交预览",
      exact: true,
    });
    const saved = {};
    assert.equal(await previewButton.isDisabled(), false);
    assert.equal(
      await page.getByText("API 暂未提供可选值", { exact: true }).count(),
      0,
    );
    for (const [id, hint] of [
      ["title", "请填写问题标题"],
      ["actual", "描述实际发生的问题表现"],
      [
        "preconditions",
        "补充安装方式、账号（如有）、网络情况、涉及的云控（如有）",
      ],
      ["steps", "每行填写一步，描述可执行的操作"],
      ["expected", "需要描述清楚应该是什么表现，越清楚越好"],
      ["customFields.hintText", "请填写必填说明"],
    ]) {
      saved[id] = await input(id).inputValue();
      await input(id).fill(" \t ");
      assert.equal(await input(id).getAttribute("placeholder"), hint);
      assert.equal(await messages(id).count(), 0);
      assert.equal(await previewButton.isDisabled(), true);
      await input(id).fill(saved[id]);
    }
    await input("actual").fill("");
    await page
      .locator("#field-actual")
      .screenshot({ path: path.join(output, "empty-actual.png") });
    await input("actual").fill(saved.actual);
    await input("steps").fill("3331111\n444\n555\n666");
    assert.equal(await messages("steps").count(), 0);
    await input("steps").fill(saved.steps);
    assert.equal(await messages("steps").count(), 0);
    for (const [id, hint] of [
      ["hintInt", "请输入整数"],
      ["hintFloat", "请输入数值"],
      ["hintTime", "例如 2026-10-10T14:30:00+08:00"],
    ]) {
      const field = `customFields.${id}`;
      assert.equal(await input(field).getAttribute("placeholder"), hint);
      assert.equal(await messages(field).count(), 0);
      await input(field).fill("invalid");
      assert.equal(await messages(field).count(), 1);
      assert.equal(await previewButton.isDisabled(), true);
      await input(field).fill("");
      assert.equal(await messages(field).count(), 0);
    }
    const firstDiscovery = page.getByRole("checkbox", {
      name: "首次发现，尚未重复",
    });
    await firstDiscovery.uncheck();
    await input("frequency.attempts").fill("");
    await input("frequency.occurrences").fill("");
    assert.equal(
      await page.locator(".frequency-row > .field > small").count(),
      0,
    );
    assert.equal(await previewButton.isDisabled(), true);
    await input("frequency.attempts").fill("1");
    await input("frequency.occurrences").fill("2");
    assert.equal(await messages("frequency.occurrences").count(), 1);
    await input("frequency.attempts").fill("3");
    assert.equal(await messages("frequency.occurrences").count(), 0);
    await firstDiscovery.check();
    const kind = page.getByRole("combobox", { name: "问题类型", exact: true });
    for (const [name, paths] of [
      [
        "网络",
        [
          "network.operation",
          "network.type",
          "network.error",
          "network.collectionStatus",
        ],
      ],
      ["数据", ["data.type", "data.count", "data.source"]],
    ]) {
      await kind.click();
      await page.getByRole("option", { name, exact: true }).click();
      for (const id of paths) {
        await input(id).fill("");
        assert.ok(await input(id).getAttribute("placeholder"));
        assert.equal(await messages(id).count(), 0);
      }
      assert.equal(await previewButton.isDisabled(), true);
    }
    await kind.click();
    await page.getByRole("option", { name: "功能", exact: true }).click();
    const logCard = page
      .locator(".attachment")
      .filter({ hasText: "logcat.log" });
    const startField = logCard
      .locator(".field")
      .filter({ hasText: "真实覆盖起点" });
    const endField = logCard
      .locator(".field")
      .filter({ hasText: "真实覆盖终点" });
    const oldStart = await startField.locator("input").inputValue();
    const oldEnd = await endField.locator("input").inputValue();
    await startField.locator("input").fill("");
    await endField.locator("input").fill("");
    await logCard
      .getByRole("button", { name: "保存覆盖时间", exact: true })
      .click();
    assert.ok(await startField.locator("input").getAttribute("placeholder"));
    assert.ok(await endField.locator("input").getAttribute("placeholder"));
    assert.equal(await logCard.locator(".field > small").count(), 0);
    assert.equal(await previewButton.isDisabled(), true);
    await startField.locator("input").fill("invalid");
    await logCard
      .getByRole("button", { name: "保存覆盖时间", exact: true })
      .click();
    assert.equal(await startField.locator(".error-text").count(), 1);
    await startField.locator("input").fill(oldStart);
    await endField.locator("input").fill(oldEnd);
    await logCard
      .getByRole("button", { name: "保存覆盖时间", exact: true })
      .click();
    assert.equal(await logCard.locator(".field > small").count(), 0);
    await page.evaluate(() => {
      window.__qa.backendIssues = [
        { field: "actual", message: "模拟主进程问题表现错误" },
      ];
    });
    await previewButton.click();
    assert.equal(
      await messages("actual").innerText(),
      "模拟主进程问题表现错误",
    );
    assert.equal(await page.locator(".preview").count(), 0);
    await page.evaluate(() => {
      window.__qa.backendIssues = [];
    });
    await input("actual").fill(saved.actual + "。");
    assert.equal(await messages("actual").count(), 0);
    await page.getByRole("button", { name: "设置", exact: true }).click();
    assert.equal(
      await page.getByRole("searchbox").getAttribute("placeholder"),
      "项目名或 Android 包名",
    );
    assert.equal(
      await page.locator(".project-package-setting > small").count(),
      0,
    );
    assert.equal(
      await page
        .getByRole("textbox", { name: "模拟项目 Android 包名" })
        .getAttribute("placeholder"),
      "主页面功能依赖此包名，如 com.example.app",
    );
    assert.deepEqual(errors, []);
    assert.equal(
      await page.evaluate(
        () => window.__qa.calls.filter((c) => c.cmd === "submit_bug").length,
      ),
      0,
    );
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "Actual React input hints with mocked IPC; no real cloud/USB",
          scenarios: [
            "six required problem inputs use hints without duplicate empty prompts and keep preview blocked",
            "dynamic required text uses an input hint",
            "nonblank reproduction steps do not use a subjective per-line length gate",
            "dynamic int/float/dateTime retain local format errors",
            "empty frequency uses hints, invalid nonempty frequency shows an error",
            "network inputs use hints and keep required validation",
            "data inputs use hints and keep required validation",
            "empty log coverage uses hints and still blocks preview",
            "invalid log coverage remains local and clears after correction",
            "backend problem error is local and prevents preview",
            "settings input hints have no duplicates",
          ],
        },
        null,
        2,
      ) + "\n",
    );
    console.log("11 input-hint scenarios passed; no real cloud/USB.");
    break checks;
  }
  if (process.env.QA_SCOPE === "frequency-row") {
    const firstDiscovery = page.getByRole("checkbox", {
      name: "首次发现，尚未重复",
    });
    const attempts = page.locator('[id="field-frequency.attempts"] input');
    const occurrences = page.locator(
      '[id="field-frequency.occurrences"] input',
    );
    const frequencyIssues = page
      .locator(".missing-list button")
      .filter({ hasText: "发生频率" });
    await firstDiscovery.check();
    assert.equal(await attempts.count(), 0);
    await firstDiscovery.uncheck();
    assert.equal(
      await attempts.getAttribute("placeholder"),
      "尝试次数应为大于 0 的整数",
    );
    assert.equal(
      await occurrences.getAttribute("placeholder"),
      "发生次数应为 1 至尝试次数之间的整数",
    );
    assert.equal(await page.locator(".frequency-row small").count(), 0);
    for (const size of [
      { width: 980, height: 700 },
      { width: 1280, height: 900 },
      { width: 1440, height: 1000 },
    ]) {
      await page.setViewportSize(size);
      const layout = await page.locator(".frequency-row").evaluate((node) => {
        const checkbox = node.querySelector(".check").getBoundingClientRect();
        const inputs = [...node.querySelectorAll(".field input")].map((input) =>
          input.getBoundingClientRect(),
        );
        return {
          aligned: inputs.every(
            (input) => Math.abs(input.bottom - checkbox.bottom) <= 1,
          ),
          distinctColumns:
            checkbox.right < inputs[0].left && inputs[0].right < inputs[1].left,
          belowDropdowns:
            node.getBoundingClientRect().top >=
            document.querySelector(".issue-select-grid").getBoundingClientRect()
              .bottom,
          noOverflow: document.documentElement.scrollWidth <= innerWidth,
        };
      });
      assert.deepEqual(layout, {
        aligned: true,
        distinctColumns: true,
        belowDropdowns: true,
        noOverflow: true,
      });
      await page.locator(".frequency-row").screenshot({
        path: path.join(output, `frequency-row-${size.width}.png`),
      });
    }
    await attempts.fill("1");
    await occurrences.fill("2");
    assert.ok((await frequencyIssues.count()) > 0);
    assert.equal(
      await page
        .getByRole("button", { name: "进入提交预览", exact: true })
        .isDisabled(),
      true,
    );
    assert.equal(await page.locator(".frequency-row .error-text").count(), 1);
    await frequencyIssues.click();
    await page.waitForFunction(() => {
      const field = document
        .getElementById("field-frequency.occurrences")
        .getBoundingClientRect();
      const header = document
        .querySelector(".page-header")
        .getBoundingClientRect();
      return field.top >= header.bottom && field.top < innerHeight;
    });
    await attempts.fill("3");
    assert.equal(await frequencyIssues.count(), 0);
    await occurrences.fill("0");
    assert.ok((await frequencyIssues.count()) > 0);
    await firstDiscovery.check();
    assert.equal(await attempts.count(), 0);
    assert.equal(await frequencyIssues.count(), 0);
    await firstDiscovery.uncheck();
    assert.equal(await attempts.inputValue(), "3");
    assert.equal(await occurrences.inputValue(), "0");
    assert.ok((await frequencyIssues.count()) > 0);
    assert.deepEqual(errors, []);
    assert.equal(
      await page.evaluate(
        () =>
          window.__qa.calls.filter((call) => call.cmd === "submit_bug").length,
      ),
      0,
    );
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "Actual React frequency controls with mocked IPC; no real cloud/USB",
          scenarios: [
            "checkbox and count inputs share a row at 980, 1280 and 1440 pixels",
            "count rules appear only as input hints",
            "invalid counts still block preview and appear in final checks",
            "final-check navigation scrolls to the occurrences input",
            "valid counts clear the frequency check",
            "first-discovery toggle hides inputs, bypasses count validation and retains values",
          ],
        },
        null,
        2,
      ) + "\n",
    );
    console.log("6 frequency-row scenarios passed; no real cloud/USB.");
    break checks;
  }
  for (const [id, name] of [
    ["participants", "参与者"],
    ["trackers", "抄送"],
  ]) {
    const control = page.getByRole("combobox", { name, exact: true });
    assert.match(await control.innerText(), /未选择/);
    await control.click();
    assert.equal(await page.getByRole("option", { selected: true }).count(), 0);
    assert.equal(
      await page.getByRole("option").count(),
      metadata.members.length,
    );
    const option = page.getByRole("option", {
      name: metadata.members[0].name,
      exact: true,
    });
    await option.click();
    assert.equal(await control.getAttribute("aria-expanded"), "true");
    assert.equal(await option.getAttribute("aria-selected"), "true");
    await control.press("Escape");
    await page.getByRole("button", { name: "保存草稿", exact: true }).click();
    assert.deepEqual(
      await page.evaluate((key) => window.__qa.drafts[0].customFields[key], id),
      [metadata.members[0].id],
    );
    await page.locator("nav button").filter({ hasText: "设置" }).click();
    await page.locator("nav button").filter({ hasText: "新建提交" }).click();
    assert.match(
      await control.innerText(),
      new RegExp(metadata.members[0].name),
    );
    await control.click();
    await page
      .getByRole("option", { name: metadata.members[0].name, exact: true })
      .click();
    assert.equal(await control.getAttribute("aria-expanded"), "true");
    assert.equal(await page.getByRole("option", { selected: true }).count(), 0);
    await control.press("Escape");
    await page.getByRole("button", { name: "保存草稿", exact: true }).click();
    await page
      .getByRole("button", { name: "刷新云效配置", exact: true })
      .click();
    assert.match(await control.innerText(), /未选择/);
    assert.deepEqual(
      await page.evaluate((key) => window.__qa.drafts[0].customFields[key], id),
      [],
    );
  }
  await page.evaluate(() => window.scrollTo(0, 0));
  assert.equal(await page.locator(".page-header").count(), 1);
  const newIssueButton = page.getByRole("button", {
    name: "＋ 新建问题",
    exact: true,
  });
  assert.equal(await newIssueButton.count(), 1);
  assert.deepEqual(
    await newIssueButton.evaluate((node) => {
      const style = getComputedStyle(node);
      return {
        background: style.backgroundColor,
        color: style.color,
      };
    }),
    { background: "rgb(40, 100, 216)", color: "rgb(255, 255, 255)" },
  );
  if (process.env.QA_SCOPE === "new-issue-blue") {
    await page.locator(".page-header").screenshot({
      path: path.join(output, "new-issue-blue.png"),
    });
    assert.deepEqual(errors, []);
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "Actual React header with mocked IPC; no real cloud/device",
          scenarios: [
            "new issue action uses the primary blue background and white text",
            "header retains the existing project, draft and action layout",
          ],
        },
        null,
        2,
      ) + "\n",
    );
    console.log("2 new-issue button style scenarios passed.");
    break checks;
  }
  const wideInitialGeometry = await page
    .locator(".page-header")
    .evaluate((node) => {
      const header = node.getBoundingClientRect();
      const card = document
        .querySelector(".issue-details-card")
        ?.getBoundingClientRect();
      return {
        top: header.top,
        bottom: header.bottom,
        cardTop: card?.top ?? 0,
      };
    });
  assert.ok(wideInitialGeometry.cardTop >= wideInitialGeometry.bottom);
  await page.evaluate(() => window.scrollTo(0, 500));
  await page.waitForFunction(() => window.scrollY >= 400);
  const wideHeaderGeometry = await page
    .locator(".page-header")
    .evaluate((node) => {
      const header = node.getBoundingClientRect();
      const card = document
        .querySelector(".issue-details-card")
        ?.getBoundingClientRect();
      return {
        top: header.top,
        bottom: header.bottom,
        cardTop: card?.top ?? 0,
      };
    });
  assert.ok(wideHeaderGeometry.top >= -1 && wideHeaderGeometry.top <= 1);
  assert.ok(Math.abs(wideHeaderGeometry.top - wideInitialGeometry.top) <= 1);
  assert.ok(wideHeaderGeometry.cardTop < wideInitialGeometry.cardTop - 300);
  await page.screenshot({
    path: path.join(output, "header-wide-scrolled.png"),
  });
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.locator("nav button").filter({ hasText: "历史提交" }).click();
  await page
    .getByRole("heading", { name: "本机提交记录", exact: true })
    .waitFor();
  assert.equal(await page.locator(".page-header").count(), 0);
  assert.equal(
    await page
      .getByRole("button", { name: "＋ 新建问题", exact: true })
      .count(),
    0,
  );
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  await page.getByRole("heading", { name: "项目配置", exact: true }).waitFor();
  assert.equal(await page.locator(".page-header").count(), 0);
  assert.equal(
    await page
      .getByRole("button", { name: "＋ 新建问题", exact: true })
      .count(),
    0,
  );
  await page.locator("nav button").filter({ hasText: "新建提交" }).click();
  await page.getByRole("heading", { name: "新建提交", exact: true }).waitFor();
  await page.setViewportSize({ width: 980, height: 700 });
  await page.evaluate(() => window.scrollTo(0, 0));
  const narrowInitialGeometry = await page
    .locator(".page-header")
    .evaluate((node) => {
      const header = node.getBoundingClientRect();
      const card = document
        .querySelector(".issue-details-card")
        ?.getBoundingClientRect();
      return {
        top: header.top,
        bottom: header.bottom,
        cardTop: card?.top ?? 0,
      };
    });
  assert.ok(narrowInitialGeometry.cardTop >= narrowInitialGeometry.bottom);
  await page.evaluate(() => window.scrollTo(0, 700));
  await page.waitForFunction(() => window.scrollY >= 500);
  const narrowHeaderGeometry = await page
    .locator(".page-header")
    .evaluate((node) => {
      const header = node.getBoundingClientRect();
      const card = document
        .querySelector(".issue-details-card")
        ?.getBoundingClientRect();
      return {
        top: header.top,
        bottom: header.bottom,
        cardTop: card?.top ?? 0,
      };
    });
  assert.ok(narrowHeaderGeometry.top >= -1 && narrowHeaderGeometry.top <= 1);
  assert.ok(
    Math.abs(narrowHeaderGeometry.top - narrowInitialGeometry.top) <= 1,
  );
  assert.ok(narrowHeaderGeometry.cardTop < narrowInitialGeometry.cardTop - 300);
  await page.screenshot({
    path: path.join(output, "header-narrow-scrolled.png"),
  });
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => window.scrollTo(0, 0));
  const problemFields = [
    ["jenkinsPackageUrl", "Jenkins包地址（必填）"],
    ["title", "问题标题（必填）"],
    ["actual", "问题表现（必填）"],
    ["bugPage", "Bug所在页面（选填）"],
    ["preconditions", "复现条件（选填）"],
    ["steps", "复现步骤（选填）"],
    ["expected", "期望表现（选填）"],
  ];
  const problemInput = (id) =>
    page.locator(`#field-${id} input, #field-${id} textarea`);
  for (const [id, label] of problemFields) {
    assert.ok(
      (await page.locator(`#field-${id} > span`).innerText()).startsWith(label),
    );
    assert.equal(
      await problemInput(id).getAttribute("required"),
      ["jenkinsPackageUrl", "title", "actual"].includes(id) ? "" : null,
    );
  }
  assert.equal(
    await page
      .locator('[id="field-event.occurredAt"], [id="field-event.sourceNote"]')
      .count(),
    0,
  );
  assert.equal(
    await page.locator("#field-title input").getAttribute("placeholder"),
    null,
  );
  assert.equal(
    await problemInput("jenkinsPackageUrl").getAttribute("placeholder"),
    "请填写 Jenkins 包地址",
  );
  assert.equal(
    await problemInput("preconditions").getAttribute("placeholder"),
    "补充安装方式、账号（如有）、网络情况、涉及的云控（如有）",
  );
  assert.equal(
    await problemInput("expected").getAttribute("placeholder"),
    "需要描述清楚应该是什么表现，越清楚越好",
  );
  for (const size of [
    { width: 1440, height: 900 },
    { width: 1280, height: 900 },
    { width: 980, height: 700 },
  ]) {
    await page.setViewportSize(size);
    const layout = await page
      .locator(".problem-fields-grid")
      .evaluate((node) => {
        const grid = node.getBoundingClientRect();
        const field = (id) =>
          document.getElementById(`field-${id}`).getBoundingClientRect();
        const jenkins = field("jenkinsPackageUrl"),
          title = field("title"),
          actual = field("actual"),
          bugPage = field("bugPage"),
          conditions = field("preconditions"),
          steps = field("steps"),
          expected = field("expected");
        return {
          jenkinsWidth: jenkins.width,
          titleWidth: title.width,
          gridWidth: grid.width,
          orderedFullRows:
            title.top >= jenkins.bottom &&
            actual.top >= title.bottom &&
            bugPage.top >= actual.bottom &&
            conditions.top >= bugPage.bottom,
          rowsAligned:
            Math.abs(expected.top - conditions.top) <= 1 &&
            steps.top >= conditions.bottom,
          noOverflow: document.documentElement.scrollWidth <= innerWidth,
        };
      });
    assert.ok(Math.abs(layout.jenkinsWidth - layout.gridWidth) <= 1);
    assert.ok(Math.abs(layout.titleWidth - layout.gridWidth) <= 1);
    assert.equal(layout.orderedFullRows, true);
    assert.equal(layout.rowsAligned, true);
    assert.equal(layout.noOverflow, true);
    await page.locator(".issue-details-card").screenshot({
      path: path.join(output, `problem-fields-${size.width}.png`),
    });
  }
  await page.setViewportSize({ width: 1280, height: 900 });
  const originalTitle = await page.locator("#field-title input").inputValue();
  await page.locator("#field-title input").fill("");
  await page.waitForTimeout(300);
  await page
    .locator(".missing-list button")
    .filter({ hasText: "问题标题" })
    .click();
  await page.waitForTimeout(500);
  assert.equal(
    await page
      .locator("#field-title")
      .evaluate((node) => node.classList.contains("validation-target-error")),
    true,
  );
  assert.equal(
    await page
      .locator(".missing-list button")
      .filter({ hasText: "问题标题" })
      .evaluate((node) => node.classList.contains("active")),
    true,
  );
  const validationColors = await page.evaluate(() => {
    const field = document.querySelector("#field-title input");
    const chip = [...document.querySelectorAll(".missing-list button")].find(
      (node) => node.textContent?.includes("问题标题"),
    );
    return {
      fieldBorder: getComputedStyle(field).borderColor,
      fieldBackground: getComputedStyle(field).backgroundColor,
      chipBackground: getComputedStyle(chip).backgroundColor,
    };
  });
  assert.equal(validationColors.fieldBorder, "rgb(212, 106, 85)");
  assert.equal(validationColors.fieldBackground, "rgb(255, 246, 243)");
  assert.equal(validationColors.chipBackground, "rgb(180, 73, 54)");
  await page.screenshot({
    path: path.join(output, "validation-target-highlight.png"),
    fullPage: false,
  });
  const targetGeometry = await page.locator("#field-title").evaluate((node) => {
    const target = node.getBoundingClientRect();
    const header = document
      .querySelector(".page-header")
      ?.getBoundingClientRect();
    return {
      top: target.top,
      bottom: header?.bottom ?? 0,
      visible: target.top < innerHeight && target.bottom > 0,
    };
  });
  assert.ok(
    targetGeometry.visible && targetGeometry.top >= targetGeometry.bottom,
  );
  await page.locator("#field-title input").fill(originalTitle);
  assert.equal(
    await page
      .locator("#field-title")
      .evaluate((node) => node.classList.contains("validation-target-error")),
    false,
  );
  const previewCard = page.locator(".attachment").first();
  await previewCard.getByRole("button", { name: "预览", exact: true }).click();
  await page.waitForFunction(() =>
    window.__qa.calls.some((call) => call.cmd === "preview_attachment"),
  );
  assert.equal(
    await page.locator(".modal, .attachment img, .attachment video").count(),
    0,
  );
  const previewArgs = await page.evaluate(
    () =>
      window.__qa.calls.find((call) => call.cmd === "preview_attachment").args,
  );
  assert.deepEqual(Object.keys(previewArgs).sort(), ["attachmentId", "id"]);
  await page.evaluate(() => (window.__qa.failPreview = true));
  await previewCard.getByRole("button", { name: "预览", exact: true }).click();
  await previewCard.getByText(/无法使用系统应用打开此附件/).waitFor();
  assert.ok(
    await previewCard
      .locator(".action-item")
      .evaluate(
        (node) =>
          node.querySelector(".error-text").getBoundingClientRect().top >=
          node.querySelector("button").getBoundingClientRect().bottom,
      ),
  );
  await page.evaluate(() => (window.__qa.failPreview = false));
  await previewCard.getByRole("button", { name: "预览", exact: true }).click();
  await page.waitForFunction(
    () => !document.querySelector(".attachment .action-item .error-text"),
  );
  assert.equal(
    await page.getByRole("button", { name: "导入附件", exact: true }).count(),
    1,
  );
  for (const name of [
    "导入日志",
    "导入截图",
    "导入 MP4",
    "导入系统报告 ZIP",
    "导入数据样例",
    "采集系统报告",
    "恢复中断窗口记录",
  ])
    assert.equal(
      await page.getByRole("button", { name, exact: true }).count(),
      0,
    );
  await page.evaluate(() => (window.__qa.cancelImport = true));
  await page.getByRole("button", { name: "导入附件", exact: true }).click();
  await page
    .locator(".notification-toast.warning", { hasText: "已取消导入附件" })
    .waitFor();
  assert.equal(
    await page
      .locator("#field-attachment-import .error-text", { hasText: "已取消" })
      .count(),
    0,
    "取消导入作为右下角提示，不显示在按钮下方",
  );
  await page.evaluate(() => (window.__qa.cancelImport = false));
  await page.evaluate(() => (window.__qa.failImport = true));
  await page.getByRole("button", { name: "导入附件", exact: true }).click();
  await page
    .locator("#field-attachment-import")
    .getByText("模拟附件导入失败", { exact: true })
    .waitFor();
  for (const width of [980, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const buttons = await page.locator(".attachment-actions").evaluate((node) =>
      [...node.querySelectorAll("button")].map((button) => {
        const { top, width, height } = button.getBoundingClientRect();
        return { top, width, height };
      }),
    );
    assert.ok(
      Math.abs(buttons[0].top - buttons[1].top) < 1,
      "evidence buttons align despite different error heights",
    );
    assert.equal(buttons[0].width, buttons[1].width);
    assert.equal(buttons[0].height, buttons[1].height);
  }
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.locator("#field-attachments").scrollIntoViewIfNeeded();
  await page
    .locator("#field-attachments")
    .screenshot({ path: path.join(output, "attachment-actions-aligned.png") });
  assert.doesNotMatch(
    await page.locator("#field-attachments").innerText(),
    /派生副本与来源回执一起选择|超过\s*50\s*MB/,
  );
  await page.evaluate(() => {
    window.__qa.failImport = false;
    window.__qa.importBatch = [
      { name: "mixed-picture.png", role: "image" },
      { name: "mixed-recording.mp4", role: "video" },
      { name: "mixed-report.zip", role: "report" },
      { name: "mixed-data.json", role: "sample" },
    ];
  });
  await page.getByRole("button", { name: "导入附件", exact: true }).click();
  await page
    .locator(".notification-toast.success", { hasText: "附件已导入" })
    .waitFor();
  for (const name of [
    "mixed-picture.png",
    "mixed-recording.mp4",
    "mixed-report.zip",
    "mixed-data.json",
  ]) {
    const card = page.locator(".attachment").filter({ hasText: name });
    await card.waitFor();
    await card.getByRole("button", { name: "预览", exact: true }).click();
    await card.getByRole("button", { name: "移除", exact: true }).click();
    await card.waitFor({ state: "detached" });
  }
  await page.evaluate(() => (window.__qa.importBatch = null));
  if (process.env.QA_SCOPE === "attachments") {
    await page.evaluate(() => (window.__qa.captures = []));
    while (await page.locator(".attachment").count()) {
      const before = await page.locator(".attachment").count();
      await page
        .locator(".attachment")
        .first()
        .getByRole("button", { name: "移除", exact: true })
        .click();
      await page.waitForFunction(
        (count) => document.querySelectorAll(".attachment").length < count,
        before,
      );
    }
    const emptyPreview = page.getByRole("button", {
      name: "进入提交预览",
      exact: true,
    });
    const emptyKind = page.getByRole("combobox", {
      name: "问题类型",
      exact: true,
    });
    assert.equal(
      await page.getByRole("checkbox", { name: /事后补采/ }).count(),
      0,
    );
    for (const name of ["功能", "UI", "多语言", "动态过程"]) {
      await emptyKind.click();
      await page.getByRole("option", { name, exact: true }).click();
      assert.equal(
        await emptyPreview.isDisabled(),
        false,
        `${name} without attachments may preview`,
      );
      await emptyPreview.click();
      await page
        .getByRole("heading", { name: "提交预览", exact: true })
        .waitFor();
      assert.equal(await page.locator(".preview li").count(), 0);
      assert.equal(
        await page.locator("#field-attachment-import .error-text").count(),
        0,
      );
      await page
        .getByRole("button", { name: "关闭提交预览", exact: true })
        .click();
    }
    await page.locator("#field-attachments").screenshot({
      path: path.join(output, "optional-empty-attachments.png"),
    });
    const calls = await page.evaluate(() =>
      window.__qa.calls.map((call) => call.cmd),
    );
    assert.ok(!calls.includes("submit_bug"));
    await server.verifySource();
    fs.writeFileSync(
      path.join(output, "result.json"),
      JSON.stringify(
        {
          passed: true,
          mode: "actual React attachment flows with mocked IPC; no real cloud/USB",
          scenarios: [
            "mixed file import and system preview requests",
            "import success and cancellation use bottom-right notices",
            "import and preview errors below their buttons",
            "equal button geometry with unequal error heights at 980/1280/1440",
            "evidence explanatory paragraph removed",
            "late capture checkbox removed; empty attachments allow preview for functional/UI/multilingual/dynamic issues",
            "empty submission preview includes no attachment and makes no remote write",
          ],
          calls,
        },
        null,
        2,
      ) + "\n",
    );
    console.log("7 focused attachment scenarios passed; no remote writes.");
    break checks;
  }
  assert.equal(
    await page.locator('[id="field-customFields.labels"]').count(),
    0,
  );
  assert.equal(
    await page.locator('[id="field-customFields.plannedStartDate"]').count(),
    0,
  );
  assert.equal(
    await page.locator('[id="field-customFields.notFixReason"]').count(),
    0,
  );
  assert.equal(
    await page.locator('[id="field-customFields.estimatedTime"]').count(),
    0,
  );
  assert.equal(
    await page.locator('[id="field-customFields.legacyHidden"]').count(),
    0,
  );
  const pendingCapture = await page.evaluate(() =>
    structuredClone(window.__qa.captures),
  );
  await page.evaluate(() => (window.__qa.captures = []));
  await page.getByRole("button", { name: "导入附件", exact: true }).click();
  // Exercise local field routing on the real React renderer.
  const titleField = page.locator('[id="field-title"]');
  const savedTitle = await titleField.locator("input").inputValue();
  await titleField.locator("input").fill("");
  assert.equal(
    await titleField.locator(".error-text, .warning-text").count(),
    0,
  );
  assert.equal(await titleField.locator(":scope > small").count(), 0);
  assert.doesNotMatch(
    await page.locator(".missing-list").innerText(),
    /标题至少|请补充/,
  );
  await titleField.locator("input").fill(savedTitle);
  for (const [id, label] of problemFields.filter(([id]) =>
    ["jenkinsPackageUrl", "title", "actual"].includes(id),
  )) {
    const input = problemInput(id);
    const saved = await input.inputValue();
    await input.fill(" \t ");
    assert.equal(
      await page
        .getByRole("button", { name: "进入提交预览", exact: true })
        .isDisabled(),
      true,
    );
    assert.ok(
      (await page
        .locator(".missing-list button")
        .filter({ hasText: label.replace("（必填）", "") })
        .count()) > 0,
    );
    if (id !== "title") {
      assert.equal(await page.locator(`#field-${id} .error-text`).count(), 0);
    }
    await input.fill(saved);
  }
  const firstDiscovery = page.getByRole("checkbox", {
    name: "首次发现，尚未重复",
  });
  await firstDiscovery.uncheck();
  await page.locator('[id="field-frequency.attempts"] input').fill("1");
  const occurrences = page.locator('[id="field-frequency.occurrences"]');
  await occurrences.locator("input").fill("2");
  assert.equal(
    await occurrences.locator("input").getAttribute("placeholder"),
    "发生次数应为 1 至尝试次数之间的整数",
  );
  assert.equal(await occurrences.locator(".error-text").count(), 1);
  await page
    .locator(".missing-list button")
    .filter({ hasText: "发生频率" })
    .click();
  await page.waitForTimeout(350);
  assert.ok(
    await occurrences.evaluate((node) => {
      const r = node.getBoundingClientRect(),
        h = document.querySelector(".page-header").getBoundingClientRect();
      return r.top >= h.bottom && r.top < innerHeight;
    }),
  );
  await firstDiscovery.check();
  assert.equal(
    await page.getByRole("checkbox", { name: /事后补采/ }).count(),
    0,
  );
  const kind = page.getByRole("combobox", { name: "问题类型", exact: true });
  await kind.click();
  await page.getByRole("option", { name: "动态过程", exact: true }).click();
  assert.doesNotMatch(
    await page.locator('[id="field-attachment-import"]').innerText(),
    /显示问题需要截图或视频/,
  );
  assert.doesNotMatch(
    await page.locator('[id="field-attachment-import"]').innerText(),
    /动态过程需要视频/,
  );
  await kind.click();
  await page.getByRole("option", { name: "功能", exact: true }).click();
  const logCard = page.locator(".attachment").filter({ hasText: "logcat.log" });
  const endField = logCard
    .locator(".field")
    .filter({ hasText: "真实覆盖终点" });
  const oldEnd = await endField.locator("input").inputValue();
  await endField.locator("input").fill("2020-01-01T00:00:00+08:00");
  await logCard
    .getByRole("button", { name: "保存覆盖时间", exact: true })
    .click();
  assert.match(await endField.innerText(), /真实覆盖终点/);
  assert.doesNotMatch(
    await page.locator('[id="field-attachment-import"]').innerText(),
    /真实覆盖/,
  );
  await endField.locator("input").fill(oldEnd);
  await logCard
    .getByRole("button", { name: "保存覆盖时间", exact: true })
    .click();
  await page.evaluate(() => {
    const q = window.__qa;
    q.profile.fieldMappings["server-subject"] = "subject";
    q.metadata.fields.push({
      id: "server-subject",
      name: "服务端标题",
      format: "string",
      type: "CustomField",
      required: false,
      options: [],
    });
    q.backendIssues = [
      { field: "customFields.server-subject", message: "模拟主进程标题错误" },
    ];
  });
  await page.getByRole("button", { name: "导入附件", exact: true }).click();
  await page.getByRole("button", { name: "进入提交预览", exact: true }).click();
  await page
    .locator(".missing-list button")
    .filter({ hasText: "服务端标题" })
    .waitFor();
  await titleField.locator("input").fill("");
  assert.equal(
    await page
      .getByRole("button", {
        name: "问题标题（1项）",
        exact: true,
      })
      .count(),
    1,
  );
  assert.equal(
    await page
      .locator(".missing-list button")
      .filter({ hasText: "服务端标题" })
      .count(),
    0,
  );
  await titleField.locator("input").fill(savedTitle);
  assert.equal(
    await titleField.locator(".error-text, .warning-text").count(),
    0,
  );
  assert.equal(await page.locator("main > .banner.error").count(), 0);
  assert.equal(await page.locator(".preview").count(), 0);
  await page.evaluate(() => (window.__qa.backendIssues = []));
  await titleField.locator("input").fill(savedTitle + "。");
  await titleField.locator("input").fill(savedTitle);
  await page.screenshot({
    path: path.join(output, "home-slim.png"),
    fullPage: true,
  });
  const previewButton = page.getByRole("button", {
    name: "进入提交预览",
    exact: true,
  });
  assert.equal(
    await page.locator(".app-device-fields").count(),
    0,
    "the device card is absent from the main page",
  );
  assert.equal(
    await page.getByRole("button", { name: "应用与设备", exact: true }).count(),
    0,
    "removed app/device dialog entry stays absent",
  );
  assert.equal(await page.locator("#field-appDevice").count(), 0);
  await previewButton.click();
  await page.getByRole("heading", { name: "提交预览", exact: true }).waitFor();
  assert.equal(
    await page.getByRole("heading", { name: "提交预览", exact: true }).count(),
    1,
  );
  const previewDialog = page.getByRole("dialog", {
    name: "提交预览",
    exact: true,
  });
  assert.equal(await page.locator("main .preview").count(), 0);
  assert.equal(
    await previewDialog
      .getByRole("checkbox", { name: /我已核对项目、描述与附件内容/ })
      .count(),
    0,
  );
  const submitBugButton = previewDialog.getByRole("button", {
    name: "提交 Bug",
  });
  assert.equal(await submitBugButton.count(), 1);
  assert.equal(await submitBugButton.isEnabled(), true);
  assert.equal(await previewDialog.locator(".app-dialog-close").count(), 1);
  const previewText = await previewDialog.locator(".preview").innerText();
  assert.match(previewText, /Jenkins包地址/);
  assert.match(previewText, /Bug所在页面/);
  assert.match(previewText, /com\.example\.mock\.MainActivity/);
  assert.match(previewText, /自动采集/);
  assert.match(
    previewText,
    /https:\/\/jenkins\.example\.test\/job\/android\/128\/artifact\/app\.apk/,
  );
  assert.match(previewText, /范围/);
  assert.doesNotMatch(previewText, /业务与范围/);
  assert.doesNotMatch(
    previewText,
    /云控|"cloud"|"applicability"|"effectiveValue"/,
  );
  assert.equal(
    await page.locator(".preview dt").filter({ hasText: "构建" }).count(),
    0,
  );
  await previewDialog.screenshot({
    path: path.join(output, "submit-preview-dialog.png"),
  });
  await page.keyboard.press("Escape");
  await previewDialog.waitFor({ state: "detached" });
  await page.waitForFunction(
    () => document.activeElement?.textContent?.trim() === "进入提交预览",
  );
  await previewButton.click();
  await page.getByRole("button", { name: "关闭提交预览", exact: true }).click();
  await page.waitForFunction(
    () => document.activeElement?.textContent?.trim() === "进入提交预览",
  );
  const bugPageField = page.locator('[id="field-bugPage"]');
  assert.match(
    await bugPageField.locator(".field-source-detail").innerText(),
    /ADB mock page/,
  );
  await bugPageField.locator("input").fill("人工确认的页面");
  await page.getByRole("button", { name: "保存草稿", exact: true }).click();
  await page.getByRole("button", { name: "刷新设备", exact: true }).click();
  assert.equal(
    await bugPageField.locator("input").inputValue(),
    "人工确认的页面",
  );
  assert.match(await bugPageField.locator(".source").innerText(), /人工补充/);
  await bugPageField.locator("input").fill("");
  await page.getByRole("button", { name: "保存草稿", exact: true }).click();
  await page.getByRole("button", { name: "刷新设备", exact: true }).click();
  assert.equal(await bugPageField.locator("input").inputValue(), "");
  assert.match(await bugPageField.locator(".source").innerText(), /人工补充/);
  await page.evaluate(() => {
    const field = window.__qa.metadata.fields.find(
      (item) => item.id === "labels",
    );
    if (field) field.required = true;
  });
  await page.getByRole("button", { name: "刷新云效配置", exact: true }).click();
  await page.getByText(/已移除字段「标签」设为必填/).waitFor();
  assert.equal(
    await page.locator('[id="field-customFields.labels"]').count(),
    1,
  );
  assert.equal(await previewButton.isDisabled(), true);
  assert.equal(
    (await page.evaluate(() => window.__qa.calls.map((c) => c.cmd))).filter(
      (cmd) => cmd === "submit_bug",
    ).length,
    0,
  );
  await page.evaluate(
    ({ pendingCapture }) => {
      const field = window.__qa.metadata.fields.find(
        (item) => item.id === "labels",
      );
      if (field) field.required = false;
      window.__qa.captures = pendingCapture;
    },
    { pendingCapture },
  );
  await page.getByRole("button", { name: "刷新云效配置", exact: true }).click();
  assert.equal(
    await page.locator("select").count(),
    0,
    "all dropdowns are custom rendered",
  );
  assert.equal(
    await page.getByText("设备采集与固定构建", { exact: true }).count(),
    0,
  );
  assert.equal(await page.locator('[id="field-app.hardening"]').count(), 0);
  assert.equal(await page.getByText("加固状态", { exact: true }).count(), 0);
  assert.equal(
    await page
      .getByRole("heading", { name: "设备有效云控", exact: true })
      .count(),
    0,
  );
  assert.equal(await page.locator('[id^="field-cloud."]').count(), 0);
  assert.equal(await page.getByText("业务方向", { exact: true }).count(), 0);
  assert.equal(await page.locator('[id="field-app.obfuscation"]').count(), 0);
  assert.equal(await page.getByText("混淆状态", { exact: true }).count(), 0);
  assert.equal(
    await page.getByText("人工补充信息来源", { exact: true }).count(),
    0,
  );
  assert.equal(await page.getByText(/Jenkins 固定构建/).count(), 0);
  assert.equal(await page.getByText(/构建清单和安装指纹验证/).count(), 0);
  assert.equal(
    await page
      .getByRole("heading", { name: "应用与设备", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await page.getByRole("button", { name: "退出登录", exact: true }).count(),
    0,
  );
  assert.equal(
    await page.getByRole("button", { name: "应用与设备", exact: true }).count(),
    0,
  );
  await page.evaluate(() => (window.__qa.failDevice = true));
  await page.getByRole("button", { name: "刷新设备", exact: true }).click();
  const deviceField = page.locator('[id="field-device.serial"]');
  await deviceField.getByText(/模拟设备读取失败/).waitFor();
  assert.equal(await page.locator("main > .banner.error").count(), 0);
  await page.evaluate(() => (window.__qa.failDevice = false));
  await page.getByRole("button", { name: "刷新设备", exact: true }).click();
  await page.waitForFunction(
    () =>
      !document
        .getElementById("field-device.serial")
        .textContent.includes("模拟设备读取失败"),
  );
  await page.getByRole("combobox", { name: "目标设备", exact: true }).click();
  await page.keyboard.press("Escape");
  assert.equal(
    await page.getByRole("dialog", { name: "应用与设备", exact: true }).count(),
    0,
    "device selection is outside the fields dialog",
  );
  await page.locator(".evidence-device-connection").screenshot({
    path: path.join(output, "device-card-custom-select.png"),
  });
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  await page.getByRole("heading", { name: "项目配置", exact: true }).waitFor();
  const updateCard = page.getByTestId("update-card");
  await updateCard.getByRole("heading", { name: "检查更新" }).waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "check_for_updates")
          .length,
    ),
    1,
    "the application silently checks GitHub once at startup",
  );
  await updateCard
    .getByRole("button", { name: "检查更新", exact: true })
    .click();
  await updateCard
    .getByText("发现新版本 v0.2.0，可直接下载安装。", { exact: true })
    .waitFor();
  assert.match(await updateCard.innerText(), /当前版本\s*v0\.1\.0/);
  assert.match(await updateCard.innerText(), /最新正式版本\s*v0\.2\.0/);
  assert.match(await updateCard.innerText(), /10\.0 MB/);
  await updateCard
    .getByRole("button", { name: "立马更新", exact: true })
    .click();
  const updateErrorDialog = page.getByRole("dialog", {
    name: "更新失败",
    exact: true,
  });
  await updateErrorDialog.waitFor();
  assert.match(await updateErrorDialog.innerText(), /模拟下载安装失败/);
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "install_update")
          .length,
    ),
    1,
  );
  await updateErrorDialog
    .getByRole("button", { name: "关闭", exact: true })
    .click();
  const debugSwitch = page.getByRole("switch", {
    name: "开启调试",
    exact: true,
  });
  assert.equal(await debugSwitch.isChecked(), false);
  assert.match(
    await page.locator(".developer-options-card").innerText(),
    /详细日志最多约 20 MB.*基础日志最多约 4 MB.*25 MB/s,
  );
  await debugSwitch.check();
  assert.equal(
    await page.evaluate(() => window.__qa.developerOptions.debugEnabled),
    true,
  );
  await updateCard
    .getByRole("button", { name: "打开 Release 页面", exact: true })
    .click();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.calls.filter((call) => call.cmd === "open_release_page")
          .length,
    ),
    1,
  );
  assert.equal(
    await page.getByRole("button", { name: "退出登录", exact: true }).count(),
    1,
  );
  assert.equal(await page.locator(".project-setting-row").count(), 3);
  assert.equal(
    await page.getByRole("checkbox", { name: "第四项目" }).count(),
    0,
  );
  await page
    .getByRole("button", { name: "展开全部（4 项）", exact: true })
    .click();
  assert.equal(await page.locator(".project-setting-row").count(), 4);
  assert.equal(
    await page.getByRole("checkbox", { name: "第四项目" }).count(),
    1,
  );
  await page
    .getByRole("button", { name: "收起（仅显示 3 项）", exact: true })
    .click();
  assert.equal(await page.locator(".project-setting-row").count(), 3);
  const settingsSearch = page.getByRole("searchbox", { name: "搜索项目" });
  await settingsSearch.fill("第二");
  assert.equal(
    await page.getByRole("checkbox", { name: "模拟项目" }).count(),
    0,
  );
  assert.equal(
    await page.getByRole("checkbox", { name: "第二项目" }).count(),
    1,
  );
  await page
    .getByRole("textbox", { name: "第二项目 Android 包名" })
    .fill("com.second.edited");
  assert.equal(
    await page
      .getByRole("textbox", { name: "第二项目 Android 包名" })
      .inputValue(),
    "com.second.edited",
  );
  await settingsSearch.fill("  COM.SECOND.EDITED  ");
  assert.equal(
    await page.getByRole("checkbox", { name: "第二项目" }).count(),
    1,
  );
  assert.equal(
    await page.getByRole("checkbox", { name: "模拟项目" }).count(),
    0,
  );
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  assert.deepEqual(
    await page.evaluate(() => window.__qa.settingsByOrg["test-org"].entries),
    [
      {
        projectId: "test-project",
        enabled: true,
        packageName: "com.example.mock",
      },
      {
        projectId: "second-project",
        enabled: false,
        packageName: "com.second.edited",
      },
      {
        projectId: "long-project",
        enabled: false,
        packageName: "",
      },
      {
        projectId: "another-project",
        enabled: false,
        packageName: "",
      },
    ],
  );
  await page
    .getByRole("button", { name: "清空搜索", exact: true })
    .first()
    .click();
  assert.equal(
    await page
      .getByRole("textbox", { name: "第二项目 Android 包名" })
      .inputValue(),
    "com.second.edited",
  );
  await settingsSearch.fill("no-such-project");
  await page
    .getByText("没有符合搜索和筛选条件的项目。", { exact: true })
    .waitFor();
  await page.getByRole("button", { name: "清空搜索", exact: true }).click();
  await page.setViewportSize({ width: 980, height: 700 });
  const settingsLayout = await page
    .locator(".project-setting-row")
    .evaluateAll((rows) =>
      rows.map((row) => {
        const rect = row.getBoundingClientRect();
        return { right: rect.right, height: rect.height };
      }),
    );
  assert.ok(
    settingsLayout.every((row) => row.right <= 980 && row.height <= 64),
  );
  assert.ok(
    await page.evaluate(
      () => document.scrollingElement.scrollWidth <= window.innerWidth,
    ),
  );
  const longRow = page.locator(".project-setting-row").filter({
    hasText: "LongProjectWithoutSpaces",
  });
  const longRowGeometry = await longRow.evaluate((row) => {
    const name = row.querySelector(".check span")?.getBoundingClientRect();
    const input = row
      .querySelector("input[aria-label]")
      ?.getBoundingClientRect();
    return {
      nameRight: name?.right ?? 0,
      inputLeft: input?.left ?? 0,
      title: row.querySelector(".check span")?.getAttribute("title"),
    };
  });
  assert.ok(longRowGeometry.nameRight <= longRowGeometry.inputLeft);
  assert.equal(
    longRowGeometry.title,
    "LongProjectWithoutSpaces_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789",
  );
  await page.screenshot({
    path: path.join(output, "settings-compact-narrow.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 1280, height: 900 });
  const packageMapping = page.getByRole("textbox", {
    name: "模拟项目 Android 包名",
  });
  await packageMapping.fill("com.other.package");
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.settingsByOrg["test-org"].entries.find(
          (item) => item.projectId === "second-project",
        )?.packageName,
    ),
    "com.second.edited",
  );
  await page.screenshot({
    path: path.join(output, "settings-project-mapping.png"),
    fullPage: true,
  });
  await page.locator("nav button").filter({ hasText: "新建提交" }).click();
  const softwareEvidence = page.getByRole("button", {
    name: "软件证据",
    exact: true,
  });
  await softwareEvidence.waitFor();
  await softwareEvidence.click();
  await page.getByText("软件证据已生成并加入附件。", { exact: true }).waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.drafts[0].attachments.filter(
          (item) => item.source === "Android Bug Desk 软件运行诊断",
        ).length,
    ),
    1,
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.__qa.settingsByOrg["test-org"].entries.find(
          (item) => item.projectId === "test-project",
        )?.packageName,
    ),
    "com.other.package",
  );
  assert.equal(await page.locator('[id="field-app.packageName"]').count(), 0);
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  const projectEnabled = page
    .locator(".project-settings-list")
    .getByRole("checkbox")
    .first();
  await projectEnabled.uncheck();
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  await page.locator("nav button").filter({ hasText: "新建提交" }).click();
  await page.getByText(/没有启用项目/).waitFor();
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  await projectEnabled.check();
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  await page
    .getByRole("textbox", { name: "模拟项目 Android 包名" })
    .fill("com.failed.save");
  await page.evaluate(() => (window.__qa.failSettingsSave = true));
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  await page.getByText(/模拟设置保存失败/).waitFor();
  assert.match(
    await page
      .getByRole("button", { name: "保存设置", exact: true })
      .locator("..")
      .innerText(),
    /模拟设置保存失败/,
  );
  assert.equal(
    await page
      .getByRole("textbox", { name: "模拟项目 Android 包名" })
      .inputValue(),
    "com.failed.save",
  );
  await page.evaluate(() => (window.__qa.failSettingsSave = false));
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  const orgSelect = page.getByRole("combobox", { name: "组织" });
  await settingsSearch.fill("第二");
  await orgSelect.click();
  await page.getByRole("option", { name: "第二组织" }).click();
  await page.getByText("第二项目").waitFor();
  assert.equal(
    await page.getByRole("searchbox", { name: "搜索项目" }).inputValue(),
    "",
  );
  await page
    .getByRole("textbox", { name: "第二项目 Android 包名" })
    .fill("com.second.project");
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  await page.locator("nav button").filter({ hasText: "新建提交" }).click();
  await page.getByRole("button", { name: "新建问题草稿", exact: true }).click();
  assert.equal(await page.locator('[id="field-versions"]').count(), 0);
  assert.equal(
    await page.getByRole("combobox", { name: "版本（可多选）" }).count(),
    0,
  );
  assert.equal(
    await page.getByRole("combobox", { name: "迭代", exact: true }).count(),
    1,
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
  assert.equal(
    await page
      .getByRole("button", { name: "恢复中断窗口记录", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await page
      .getByRole("button", { name: "采集系统报告", exact: true })
      .count(),
    0,
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
  assert.equal(await redacted.locator("input[type=checkbox]").count(), 0);
  const textReceipt = page
    .locator(".attachment")
    .filter({ hasText: "text-receipt.zip" });
  await textReceipt.getByRole("button", { name: "移除", exact: true }).click();
  await page
    .getByRole("alert")
    .filter({ hasText: "来源回执须与关联副本一起保留" })
    .waitFor();
  assert.equal(await textReceipt.count(), 1);
  assert.equal(await textReceipt.locator("input[type=checkbox]").count(), 0);
  await page.screenshot({
    path: path.join(output, "evidence-mock.png"),
    fullPage: true,
  });
  await redacted.getByRole("button", { name: "移除", exact: true }).click();
  await page.waitForFunction(
    () =>
      !window.__qa.drafts[0].attachments.some(
        (a) => a.name === "redacted.log" || a.name === "text-receipt.zip",
      ),
  );
  assert.equal(await windowCard.count(), 1);
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  await page.getByRole("heading", { name: "项目配置", exact: true }).waitFor();
  await page.getByRole("button", { name: "退出登录", exact: true }).click();
  await page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  await page.evaluate(() => {
    window.__qa.settingsByOrg["test-org"] = { entries: [] };
    window.__qa.drafts = [];
    window.__qa.profile = {
      ...window.__qa.profile,
      organizationId: "unconfigured",
      projectId: "unconfigured",
    };
  });
  await page.evaluate(() => (window.__qa.failLogin = true));
  await page.getByRole("tab", { name: "填写 PAT", exact: true }).click();
  await page.locator("input[type=password]").fill("mock-invalid");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  const tokenError = page.locator(".login-form .field .error-text");
  await tokenError.getByText("模拟令牌无效", { exact: true }).waitFor();
  assert.ok(
    await tokenError.evaluate(
      (node) =>
        node.getBoundingClientRect().top >=
        node.parentElement.querySelector("input").getBoundingClientRect()
          .bottom,
    ),
  );
  await page.evaluate(() => (window.__qa.failLogin = false));
  const freshCallStart = await page.evaluate(() => window.__qa.calls.length);
  await page.locator("input[type=password]").fill("mock-only");
  await page.getByRole("button", { name: "确定", exact: true }).click();
  await page.getByText(/没有启用项目/).waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "＋ 新建问题", exact: true })
      .isDisabled(),
    true,
  );
  const freshCalls = await page.evaluate(
    (start) => window.__qa.calls.slice(start).map((call) => call.cmd),
    freshCallStart,
  );
  assert.equal(freshCalls.includes("select_project"), false);
  assert.equal(freshCalls.includes("new_draft"), false);
  await page.locator("nav button").filter({ hasText: "设置" }).click();
  await page.getByRole("heading", { name: "项目配置", exact: true }).waitFor();
  assert.equal(
    await page.getByRole("checkbox", { name: "模拟项目" }).isChecked(),
    false,
  );
  assert.equal(
    await page.getByRole("checkbox", { name: "第二项目" }).isChecked(),
    false,
  );
  await page.getByRole("searchbox", { name: "搜索项目" }).fill("第二");
  await page.getByRole("checkbox", { name: "第二项目" }).check();
  await page.getByRole("button", { name: "保存设置", exact: true }).click();
  await page.locator("nav button").filter({ hasText: "新建提交" }).click();
  await page.getByText("第二项目").waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "＋ 新建问题", exact: true })
      .isDisabled(),
    false,
  );
  await page.evaluate(() => {
    window.__qa.authFailure = true;
  });
  await page.locator("nav button").filter({ hasText: "历史提交" }).click();
  await page.getByRole("tab", { name: "云效历史 Bug", exact: true }).click();
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
          "unconfigured login selects first real project and creates draft",
          "participants and cc ignore prior selection memory and start empty",
          "participants and cc preserve explicit saved selections and remain empty after clearing and metadata refresh",
          "removed application/device dialog and manual field entry stay absent",
          "homepage removes device collection, business, manual source, and build controls",
          "cloud card and preview data are removed with no cloud input required",
          "excluded dynamic aliases, estimated time, and hidden optional fields stay absent",
          "removed legacy fields are absent while actual preview retains range data",
          "removed required field shows configuration error and blocks preview",
          "homepage header stays fixed while wide and narrow pages scroll",
          "history and settings hide homepage header and new-draft action",
          "fixed header preserves missing-field error navigation",
          "all attachment previews request the system application using only draft and attachment ids",
          "one import entry accepts mixed formats and shows import and system preview errors at their controls",
          "evidence buttons align with different error heights at three viewport widths; explanatory paragraph is removed",
          "removed recovery and system-report dialog actions stay absent",
          "new projects default unchecked while saved enabled state and package persist",
          "settings search filters project names and package mappings without dropping hidden rows",
          "empty settings search result can be cleared",
          "compact project settings rows fit narrow viewport",
          "fresh empty settings keep project selection and draft creation gated until one project is enabled",
          "settings package mapping synchronizes the hidden collected package identity",
          "Jenkins package address, title, and actual behavior are required while the remaining problem fields are optional; manual time inputs are absent",
          "Jenkins package address and title each span a full row across three viewport sizes",
          "Bug page shows source details and manual values or manual clearing survive device refresh",
          "blank problem fields block preview and navigate to the corresponding field",
          "field errors use Chinese navigation labels without summary duplication",
          "frequency errors navigate locally and late capture does not require attachments",
          "native title alias backend errors block preview and navigate to title without inline orange copy",
          "submission preview and direct submit action share a closable modal with focus return; no duplicate confirmation checkbox remains",
          "image and dynamic types do not require media; selected log coverage errors stay local",
          "device read failure stays below device selector and clears after retry",
          "login errors render below the PAT input",
          "nickname from mock user",
          "device discovery reads foreground activity and merged app/device fields",
          "bound session disables device refresh",
          "homepage removes versions selector and retains sprint selector",
          "settings page owns organization and logout controls",
          "settings update card checks the GitHub Release list only on demand and opens the fixed release page",
          "developer diagnostics persist a custom switch and expose one software-evidence action with explicit disk limits",
          "project settings save filters homepage projects",
          "all projects disabled shows recovery banner",
          "failed settings save preserves edited values and applied state",
          "organization switch reloads scoped project settings",
          "pending capture blocks preview",
          "local refresh preserves unsaved title",
          "range/time/uncertainty/gap display",
          "redaction clears the replacement input and creates its receipt",
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
  const report = JSON.parse(
    fs.readFileSync(path.join(output, "result.json"), "utf8"),
  );
  console.log(
    `${report.scenarios.length} renderer integration scenarios passed; no remote writes.`,
  );
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

// Actual React startup/login interactions with mock IPC, without OS credentials or cloud writes.
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
const output = path.resolve(process.env.QA_OUTPUT || "artifacts/login-auth");
fs.mkdirSync(output, { recursive: true });
const server = await startBrowserCheck(output);
let browser;
try {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROME_PATH
      ? { executablePath: process.env.CHROME_PATH }
      : {}),
  });
  const errors = [];
  const scenarios = [];
  const createPage = async ({
    remembered = false,
    method = "oauth",
    checkStatus = "valid",
    deferredCheck = false,
    deferredWorkspace = false,
    oauthStatus = "valid",
    deferredOAuth = false,
    viewport = { width: 980, height: 700 },
  } = {}) => {
    const context = await browser.newContext({ viewport });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.route("**/*", (route) =>
      new URL(route.request().url()).hostname === "127.0.0.1"
        ? route.continue()
        : route.abort(),
    );
    await page.addInitScript(
      ({
        remembered,
        method,
        checkStatus,
        deferredCheck,
        deferredWorkspace,
        oauthStatus,
        deferredOAuth,
      }) => {
        window.isTauri = true;
        const qa = (window.__authQA = {
          calls: [],
          authorized: false,
          user: null,
          remembered,
          method,
          checkResolved: false,
          loginFlashed: false,
          workspacePending: false,
          oauthPending: false,
        });
        const user = {
          id: "mock-user",
          name: "模拟用户",
          nickName: "模拟用户",
          lastOrganization: "",
        };
        const result = (status) => ({
          status,
          user: status === "valid" ? user : null,
          message:
            status === "invalid"
              ? "保存的登录已失效（模拟）"
              : status === "unverified"
                ? "暂时无法连接云效（模拟）"
                : status === "missing"
                  ? "请选择登录方式"
                  : "校验成功（模拟）",
        });
        new MutationObserver(() => {
          if (
            remembered &&
            !qa.checkResolved &&
            document.querySelector(".login-form")
          )
            qa.loginFlashed = true;
        }).observe(document, { childList: true, subtree: true });
        window.__TAURI_INTERNALS__ = {
          convertFileSrc: () => "",
          invoke: async (cmd, args = {}) => {
            qa.calls.push({
              cmd,
              ...(cmd === "login" || cmd === "oauth_login"
                ? { remember: args.remember }
                : {}),
            });
            if (cmd === "bootstrap")
              return {
                profile: {
                  name: "请选择项目",
                  edition: "central",
                  organizationId: "unconfigured",
                  projectId: "unconfigured",
                  requiredFieldIds: [],
                  fieldMappings: {},
                  enableVersions: true,
                  enableSprints: true,
                  attachmentWarningMb: 50,
                },
                authorized: qa.authorized,
                user: qa.user,
                rememberLogin: qa.remembered,
                loginMethod: qa.method,
                metadata: null,
                adbPath: null,
                drafts: [],
                submissions: [],
                sessions: [],
                captures: [],
              };
            if (cmd === "check_auth") {
              if (deferredCheck)
                await new Promise((resolve) => {
                  qa.resolveCheck = resolve;
                });
              qa.checkResolved = true;
              qa.authorized = checkStatus === "valid";
              qa.user = qa.authorized ? user : null;
              return result(checkStatus);
            }
            if (cmd === "list_organizations") {
              if (deferredWorkspace) {
                qa.workspacePending = true;
                await new Promise((resolve) => {
                  qa.resolveWorkspace = resolve;
                });
                qa.workspacePending = false;
              }
              return [];
            }
            if (cmd === "login") {
              qa.authorized = true;
              qa.user = user;
              qa.remembered = args.remember;
              qa.method = "pat";
              return result("valid");
            }
            if (cmd === "oauth_login") {
              qa.oauthPending = true;
              if (deferredOAuth)
                await new Promise((resolve) => {
                  qa.resolveOAuth = resolve;
                });
              qa.oauthPending = false;
              qa.authorized = oauthStatus === "valid";
              qa.user = qa.authorized ? user : null;
              if (qa.authorized) {
                qa.remembered = args.remember;
                qa.method = "oauth";
              }
              return result(oauthStatus);
            }
            if (cmd === "cancel_oauth_login") {
              qa.oauthCancelled = true;
              return;
            }
            if (cmd === "clear_auth") {
              qa.remembered = false;
              qa.authorized = false;
              qa.user = null;
              return;
            }
            throw Error(`Unexpected IPC: ${cmd}`);
          },
        };
      },
      {
        remembered,
        method,
        checkStatus,
        deferredCheck,
        deferredWorkspace,
        oauthStatus,
        deferredOAuth,
      },
    );
    await page.goto(server.baseUrl);
    return page;
  };
  const home = (p) =>
    p.getByRole("heading", { name: "新建提交", exact: true }).waitFor();
  const login = (p) =>
    p.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  const calls = (p) => p.evaluate(() => window.__authQA.calls);
  const restored = await createPage({
    remembered: true,
    method: "pat",
    deferredCheck: true,
    deferredWorkspace: true,
  });
  await restored.getByText("正在恢复登录…", { exact: true }).waitFor();
  assert.equal(await restored.locator(".login-form").count(), 0);
  await restored.screenshot({ path: path.join(output, "restoring.png") });
  await restored.waitForFunction(
    () => typeof window.__authQA.resolveCheck === "function",
  );
  await restored.evaluate(() => window.__authQA.resolveCheck());
  await restored.waitForFunction(() => window.__authQA.workspacePending);
  assert.equal(await restored.locator(".login-form").count(), 0);
  assert.equal(
    await restored.getByText("打开登录页…", { exact: true }).count(),
    0,
  );
  await restored.evaluate(() => window.__authQA.resolveWorkspace());
  await home(restored);
  assert.equal(
    await restored.evaluate(() => window.__authQA.loginFlashed),
    false,
  );
  assert.equal(
    await restored.locator(".notification-toast.loading").count(),
    0,
  );
  assert.equal(
    (await calls(restored)).filter((c) => c.cmd === "check_auth").length,
    1,
  );
  scenarios.push(
    "remembered PAT restores without rendering login, including delayed workspace; no stale login toast",
  );

  const oauthRestored = await createPage({
    remembered: true,
    method: "oauth",
    deferredCheck: true,
  });
  await oauthRestored.getByText("正在恢复登录…", { exact: true }).waitFor();
  await oauthRestored.waitForFunction(
    () => typeof window.__authQA.resolveCheck === "function",
  );
  await oauthRestored.evaluate(() => window.__authQA.resolveCheck());
  await home(oauthRestored);
  assert.equal(
    await oauthRestored.evaluate(() => window.__authQA.loginFlashed),
    false,
  );
  assert.equal(
    (await calls(oauthRestored)).filter((c) => c.cmd === "oauth_login").length,
    0,
  );
  scenarios.push(
    "remembered OAuth restores silently without opening browser again",
  );

  const manual = await createPage();
  await login(manual);
  assert.equal(
    await manual
      .getByRole("tab", { name: "授权登录", exact: true })
      .getAttribute("aria-selected"),
    "true",
  );
  assert.equal(await manual.locator("input[type=password]").count(), 0);
  assert.equal(
    (await calls(manual)).filter((c) => c.cmd === "check_auth").length,
    0,
  );
  await manual.screenshot({ path: path.join(output, "oauth-tab.png") });
  await manual.getByRole("tab", { name: "填写 PAT", exact: true }).click();
  await manual.locator("input[type=password]").fill("mock-only-pat");
  await manual.getByRole("checkbox", { name: "记住登录", exact: true }).check();
  await manual.screenshot({ path: path.join(output, "pat-tab.png") });
  await manual.getByRole("button", { name: "确定", exact: true }).click();
  await home(manual);
  assert.deepEqual(
    (await calls(manual)).find((c) => c.cmd === "login"),
    { cmd: "login", remember: true },
  );
  scenarios.push(
    "authorization and PAT tabs switch; PAT login preserves remember choice",
  );

  const granted = await createPage({ deferredOAuth: true });
  await login(granted);
  await granted
    .getByRole("checkbox", { name: "记住登录", exact: true })
    .check();
  await granted
    .getByRole("button", { name: "使用云效账号授权", exact: true })
    .click();
  await granted
    .getByRole("button", { name: "等待浏览器授权…", exact: true })
    .waitFor();
  assert.equal(
    await granted
      .getByRole("button", { name: "等待浏览器授权…", exact: true })
      .isDisabled(),
    true,
  );
  await granted.screenshot({ path: path.join(output, "oauth-pending.png") });
  await granted.evaluate(() => window.__authQA.resolveOAuth());
  await home(granted);
  assert.deepEqual(
    (await calls(granted)).find((c) => c.cmd === "oauth_login"),
    { cmd: "oauth_login", remember: true },
  );
  assert.equal(
    (await calls(granted)).filter((c) => c.cmd === "login").length,
    0,
  );
  scenarios.push(
    "browser authorization has pending/cancel state and enters home on success",
  );

  const expired = await createPage({
    remembered: true,
    method: "pat",
    checkStatus: "invalid",
    deferredCheck: true,
  });
  await expired.getByText("正在恢复登录…", { exact: true }).waitFor();
  await expired.waitForFunction(
    () => typeof window.__authQA.resolveCheck === "function",
  );
  await expired.evaluate(() => window.__authQA.resolveCheck());
  await login(expired);
  await expired
    .getByText("保存的登录已失效（模拟）", { exact: true })
    .waitFor();
  assert.equal(
    await expired
      .getByRole("checkbox", { name: "记住登录", exact: true })
      .isChecked(),
    false,
  );
  assert.equal(await expired.locator(".page-header").count(), 0);
  assert.equal(
    (await calls(expired)).filter((c) => c.cmd === "clear_auth").length,
    1,
  );
  scenarios.push(
    "expired restored credential shows login only after validation and clears remembered login",
  );

  const unavailable = await createPage({
    remembered: true,
    method: "oauth",
    checkStatus: "unverified",
  });
  await login(unavailable);
  await unavailable
    .getByText("暂时无法连接云效（模拟）", { exact: true })
    .waitFor();
  assert.equal(
    await unavailable
      .getByRole("checkbox", { name: "记住登录", exact: true })
      .isChecked(),
    true,
  );
  assert.equal(
    (await calls(unavailable)).filter((c) => c.cmd === "clear_auth").length,
    0,
  );
  scenarios.push(
    "temporary restore failure preserves saved OAuth credentials and displays an actionable error",
  );

  const cancelled = await createPage({ deferredOAuth: true });
  await login(cancelled);
  await cancelled
    .getByRole("button", { name: "使用云效账号授权", exact: true })
    .click();
  await cancelled
    .getByRole("button", { name: "等待浏览器授权…", exact: true })
    .waitFor();
  await cancelled.getByRole("tab", { name: "填写 PAT", exact: true }).click();
  await cancelled.locator("input[type=password]").waitFor();
  await cancelled.waitForFunction(() => window.__authQA.oauthCancelled);
  await cancelled.evaluate(() => window.__authQA.resolveOAuth());
  await cancelled.waitForFunction(() => !window.__authQA.oauthPending);
  assert.equal(await cancelled.locator(".page-header").count(), 0);
  assert.equal(
    (await calls(cancelled)).filter((c) => c.cmd === "list_organizations")
      .length,
    0,
  );
  scenarios.push(
    "switching to PAT cancels OAuth and ignores a late completion",
  );

  const denied = await createPage({ oauthStatus: "invalid" });
  await login(denied);
  await denied
    .getByRole("button", { name: "使用云效账号授权", exact: true })
    .click();
  await denied
    .getByRole("alert")
    .filter({ hasText: "保存的登录已失效（模拟）" })
    .waitFor();
  assert.equal(await denied.locator(".page-header").count(), 0);
  assert.equal(
    await denied
      .getByRole("button", { name: "使用云效账号授权", exact: true })
      .isEnabled(),
    true,
  );
  scenarios.push("OAuth denial remains beside its action and permits retry");
  for (const width of [980, 1280, 1440]) {
    const p = await createPage({ viewport: { width, height: 900 } });
    await login(p);
    const geometry = await p.locator(".login-card").evaluate((n) => {
      const card = n.getBoundingClientRect();
      return {
        width: card.width,
        scroll: document.documentElement.scrollWidth,
        viewport: innerWidth,
        contentInside: [...n.querySelectorAll(".oauth-login > *")].every(
          (child) =>
            child.getBoundingClientRect().right <= card.right - 30 &&
            child.scrollWidth <= child.clientWidth + 1,
        ),
      };
    });
    assert.ok(
      geometry.width <= 440 &&
        geometry.scroll <= geometry.viewport &&
        geometry.contentInside,
    );
    await p.screenshot({ path: path.join(output, `login-${width}.png`) });
  }
  scenarios.push(
    "both login tabs fit supported window widths without overflow",
  );
  assert.deepEqual(errors, []);
  await server.verifySource();
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "actual React with mock IPC; no system credentials or cloud writes",
        scenarios,
      },
      null,
      2,
    ),
  );
  console.log(`${scenarios.length} startup/OAuth/PAT React scenarios passed.`);
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

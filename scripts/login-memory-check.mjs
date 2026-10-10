// Actual React interactions with mock IPC. No OS credentials or cloud calls.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import {
  startBrowserCheck,
  recordBrowserFailure,
} from "./browser-check-support.mjs";
import { pathToFileURL } from "node:url";

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const output = path.resolve(process.env.QA_OUTPUT || "artifacts/login-memory");
fs.mkdirSync(output, { recursive: true });
const server = await startBrowserCheck(output);
const profile = JSON.parse(
  fs.readFileSync("tests/fixtures/metadata.json"),
).profile;
let browser;
try {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROME_PATH
      ? { executablePath: process.env.CHROME_PATH }
      : {}),
  });
  const errors = [];
  const createPage = async (
    remembered = false,
    status = "valid",
    loginStatus = "valid",
  ) => {
    const context = await browser.newContext({
      viewport: { width: 980, height: 700 },
    });
    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(String(error)));
    await page.route("**/*", (route) =>
      new URL(route.request().url()).hostname === "127.0.0.1"
        ? route.continue()
        : route.abort(),
    );
    await page.addInitScript(
      ({ profile, remembered, status, loginStatus }) => {
        if (localStorage.getItem("mock-remember-login") === null)
          localStorage.setItem("mock-remember-login", String(remembered));
        const q = (window.__loginQA = {
          calls: [],
          authorized: false,
          user: null,
        });
        window.isTauri = true;
        const user = {
          id: "mock-user",
          name: "模拟用户",
          nickName: "模拟用户",
          lastOrganization: "",
        };
        const result = (nextStatus) => {
          q.authorized = nextStatus === "valid";
          q.user = q.authorized ? user : null;
          return {
            status: nextStatus,
            user: q.user,
            message:
              nextStatus === "invalid"
                ? "登录令牌已失效（模拟）"
                : nextStatus === "unverified"
                  ? "暂时无法校验登录（模拟）"
                  : "校验成功（模拟）",
          };
        };
        window.__TAURI_INTERNALS__ = {
          convertFileSrc: () => "",
          invoke: async (cmd, args = {}) => {
            q.calls.push({
              cmd,
              ...(cmd === "login" ? { remember: args.remember } : {}),
            });
            if (cmd === "bootstrap")
              return {
                profile: {
                  ...profile,
                  organizationId: "unconfigured",
                  projectId: "unconfigured",
                },
                authorized: q.authorized,
                user: q.user,
                loginMethod: "pat",
                rememberLogin:
                  localStorage.getItem("mock-remember-login") === "true",
                metadata: null,
                adbPath: null,
                drafts: [],
                submissions: [],
                sessions: [],
                captures: [],
              };
            if (cmd === "login") {
              const response = result(loginStatus);
              if (response.status === "valid")
                localStorage.setItem(
                  "mock-remember-login",
                  String(args.remember),
                );
              return response;
            }
            if (cmd === "check_auth") return result(status);
            if (cmd === "clear_auth") {
              localStorage.setItem("mock-remember-login", "false");
              q.authorized = false;
              q.user = null;
              return;
            }
            if (cmd === "list_organizations") return [];
            throw Error(`Unexpected IPC: ${cmd}`);
          },
        };
      },
      { profile, remembered, status, loginStatus },
    );
    await page.goto(server.baseUrl);
    return page;
  };
  const home = (page) =>
    page.getByRole("heading", { name: "新建提交", exact: true }).waitFor();
  const login = (page) =>
    page.getByRole("heading", { name: "登录云效", exact: true }).waitFor();
  const calls = (page) => page.evaluate(() => window.__loginQA.calls);
  const submit = async (page, remember) => {
    await page
      .getByRole("checkbox", { name: "记住登录", exact: true })
      .setChecked(remember);
    await page
      .locator(".login-form input[type=password]")
      .fill("mock-only-token");
    await page.getByRole("button", { name: "确定", exact: true }).click();
  };

  const unchecked = await createPage();
  await login(unchecked);
  assert.equal(
    await unchecked.getByRole("checkbox", { name: "记住登录" }).isChecked(),
    false,
  );
  assert.deepEqual(await calls(unchecked), [{ cmd: "bootstrap" }]);
  await unchecked.screenshot({ path: path.join(output, "login-checkbox.png") });
  await submit(unchecked, false);
  await home(unchecked);
  assert.deepEqual(
    (await calls(unchecked)).find((call) => call.cmd === "login"),
    { cmd: "login", remember: false },
  );
  await unchecked.reload();
  await login(unchecked);
  assert.deepEqual(await calls(unchecked), [{ cmd: "bootstrap" }]);

  const checked = await createPage();
  await login(checked);
  await submit(checked, true);
  await home(checked);
  assert.deepEqual(
    (await calls(checked)).find((call) => call.cmd === "login"),
    { cmd: "login", remember: true },
  );
  await checked.reload();
  await home(checked);
  assert.equal(
    (await calls(checked)).filter((call) => call.cmd === "check_auth").length,
    1,
  );
  assert.equal(
    (await calls(checked)).filter((call) => call.cmd === "login").length,
    0,
  );
  await checked.locator("nav button").filter({ hasText: "设置" }).click();
  await checked.getByRole("button", { name: "退出登录", exact: true }).click();
  await login(checked);
  assert.equal(
    await checked.getByRole("checkbox", { name: "记住登录" }).isChecked(),
    false,
  );
  await checked.reload();
  await login(checked);
  assert.deepEqual(await calls(checked), [{ cmd: "bootstrap" }]);

  const expired = await createPage(true, "invalid");
  await expired.getByText("登录令牌已失效（模拟）", { exact: true }).waitFor();
  assert.equal(
    (await calls(expired)).filter((call) => call.cmd === "clear_auth").length,
    1,
  );
  assert.equal(
    await expired.getByRole("checkbox", { name: "记住登录" }).isChecked(),
    false,
  );
  assert.equal(await expired.locator("input[type=password]").inputValue(), "");
  assert.equal(await expired.locator(".page-header").count(), 0);
  const geometry = await expired
    .getByText("登录令牌已失效（模拟）", { exact: true })
    .evaluate((node) => ({
      errorTop: node.getBoundingClientRect().top,
      inputBottom: node.parentElement
        .querySelector("input")
        .getBoundingClientRect().bottom,
    }));
  assert.ok(geometry.errorTop >= geometry.inputBottom);

  const unavailable = await createPage(true, "unverified");
  await unavailable
    .getByText("暂时无法校验登录（模拟）", { exact: true })
    .waitFor();
  assert.equal(
    (await calls(unavailable)).filter((call) => call.cmd === "clear_auth")
      .length,
    0,
  );
  assert.equal(
    await unavailable.getByRole("checkbox", { name: "记住登录" }).isChecked(),
    true,
  );
  assert.equal(await unavailable.locator(".page-header").count(), 0);
  await submit(unavailable, false);
  await home(unavailable);
  await unavailable.reload();
  await login(unavailable);
  assert.deepEqual(await calls(unavailable), [{ cmd: "bootstrap" }]);

  const failed = await createPage(false, "valid", "invalid");
  await login(failed);
  await submit(failed, true);
  await failed.getByText("登录令牌已失效（模拟）", { exact: true }).waitFor();
  await failed.reload();
  await login(failed);
  assert.deepEqual(await calls(failed), [{ cmd: "bootstrap" }]);
  assert.deepEqual(errors, []);
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        mode: "actual React with mock IPC; no OS credentials or cloud calls",
        scenarios: [
          "default unchecked",
          "unchecked login stays manual after restart",
          "checked login auto-validates after restart",
          "logout clears remembered login",
          "expired saved token returns to manual login",
          "temporary failure preserves opt-in and supports unchecked login",
          "failed login does not save preference",
        ],
      },
      null,
      2,
    ),
  );
  process.stdout.write(
    "7 login-memory integration scenarios passed; no real credentials or cloud calls.\n",
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

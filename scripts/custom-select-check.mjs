// Exercise the actual custom select inside form labels and nested scroll areas.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import {
  startBrowserCheck,
  recordBrowserFailure,
} from "./browser-check-support.mjs";
import { pathToFileURL } from "node:url";
const { chromium, webkit } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href
    : "playwright"
);
const engineName = process.env.BROWSER_ENGINE || "chromium";
const engine = { chromium, webkit }[engineName];
assert.ok(engine, `Unsupported browser engine: ${engineName}`);
const output = path.resolve(process.env.QA_OUTPUT || "artifacts/custom-select");
fs.mkdirSync(output, { recursive: true });
const server = await startBrowserCheck(output);
let browser;
try {
  browser = await engine.launch({
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
  await page.goto(server.baseUrl);
  await page.evaluate(async () => {
    // Reuse Vite's loaded URLs, including their cache key, so hooks share React.
    const moduleUrl = (file) =>
      performance
        .getEntriesByType("resource")
        .find((entry) => new URL(entry.name).pathname.endsWith(`/${file}`))
        .name;
    const React = (await import(moduleUrl("react.js"))).default;
    const { createRoot } = (await import(moduleUrl("react-dom_client.js")))
      .default;
    const { CustomSelect } = await import("/src/renderer/custom-select.tsx");
    document.getElementById("root").style.display = "none";
    document.body.style.overflow = "auto";
    document.body.style.paddingRight = "7px";
    const host = document.createElement("div");
    host.id = "select-check";
    document.body.append(host);
    const options = Array.from({ length: 60 }, (_, i) => ({
      id: String(i),
      name: `选项 ${i}`,
    }));
    const h = React.createElement;
    function Demo() {
      const [single, setSingle] = React.useState("0"),
        [multi, setMulti] = React.useState(["0"]),
        [kind, setKind] = React.useState("功能"),
        [nested, setNested] = React.useState(true),
        [disabled, setDisabled] = React.useState(false),
        [mounted, setMounted] = React.useState(true);
      window.__selectQA = {
        setDisabled,
        setMounted,
        setKind,
        setNested,
        single,
        multi,
      };
      return h(
        "div",
        { style: { margin: "100px 280px", minHeight: "2400px" } },
        h(
          "div",
          {
            id: "nested-scroll",
            style: {
              height: "500px",
              overflow: "auto",
              border: "1px solid #ccc",
            },
          },
          h(
            "div",
            { style: { minHeight: "1400px", padding: "20px" } },
            mounted &&
              h(
                "label",
                { className: "field", id: "single-field" },
                h("span", {}, "单选"),
                h(CustomSelect, {
                  label: "单选",
                  value: single,
                  options,
                  onChange: setSingle,
                  disabled,
                }),
              ),
            h(
              "label",
              { className: "field", id: "multi-field" },
              h("span", {}, "多选"),
              h(CustomSelect, {
                label: "多选",
                value: multi,
                options,
                onChange: setMulti,
                multiple: true,
              }),
            ),
          ),
        ),
        h("button", { id: "outside", type: "button" }, "点击外部"),
        h(
          "div",
          { id: "scroll-cases", style: { marginTop: "1000px" } },
          h(
            "div",
            {
              id: "dynamic-scroll",
              style: {
                height: nested ? "500px" : "auto",
                overflow: nested ? "auto" : "visible",
              },
            },
            h(
              "div",
              { style: { minHeight: "1800px", paddingTop: "500px" } },
              kind !== "功能" &&
                h("div", {
                  id: "kind-details",
                  style: { height: kind === "网络" ? "240px" : "140px" },
                }),
              h(
                "label",
                { className: "field" },
                h("span", {}, "问题类型"),
                h(CustomSelect, {
                  label: "问题类型",
                  value: kind,
                  options: ["功能", "网络", "数据"].map((name) => ({
                    id: name,
                    name,
                  })),
                  onChange: setKind,
                }),
              ),
            ),
          ),
          h(
            "label",
            { className: "field", style: { marginTop: "50px" } },
            h("span", {}, "页面单选"),
            h(CustomSelect, {
              label: "页面单选",
              value: single,
              options,
              onChange: setSingle,
            }),
          ),
          h(
            "label",
            { className: "field" },
            h("span", {}, "页面多选"),
            h(CustomSelect, {
              label: "页面多选",
              value: multi,
              options,
              onChange: setMulti,
              multiple: true,
            }),
          ),
          h("div", { style: { height: "1200px" } }),
        ),
      );
    }
    createRoot(host).render(h(Demo));
  });
  const single = page.getByRole("combobox", { name: "单选", exact: true });
  const multi = page.getByRole("combobox", { name: "多选", exact: true });
  await single.waitFor();
  const original = await page.evaluate(() => ({
    overflow: document.body.style.overflow,
    padding: document.body.style.paddingRight,
  }));
  await single.click();
  await page
    .getByRole("option", { name: "选项 1", exact: true })
    .locator("span")
    .first()
    .click();
  assert.equal(await single.getAttribute("aria-expanded"), "false");
  assert.equal(await page.getByRole("listbox").count(), 0);
  assert.equal(await page.evaluate(() => window.__selectQA.single), "1");
  await single.click();
  await page
    .getByRole("option", { name: "选项 1", exact: true })
    .locator("span")
    .first()
    .click();
  assert.equal(
    await single.getAttribute("aria-expanded"),
    "false",
    "selecting the current single value also closes",
  );
  await multi.click();
  await page
    .getByRole("option", { name: "选项 1", exact: true })
    .locator("span")
    .first()
    .click();
  assert.equal(await multi.getAttribute("aria-expanded"), "true");
  assert.deepEqual(await page.evaluate(() => window.__selectQA.multi), [
    "0",
    "1",
  ]);
  await page.getByRole("option", { name: "选项 1", exact: true }).click();
  assert.equal(await multi.getAttribute("aria-expanded"), "true");
  assert.deepEqual(await page.evaluate(() => window.__selectQA.multi), ["0"]);
  await multi.press("Escape");
  await single.click();
  const menu = page.getByRole("listbox");
  await menu.hover();
  const before = await page.evaluate(() => ({
    page: scrollY,
    parent: document.getElementById("nested-scroll").scrollTop,
  }));
  await page.mouse.wheel(0, 400);
  await page.waitForFunction(
    () => document.querySelector(".custom-select-menu").scrollTop > 100,
  );
  assert.equal(await page.evaluate(() => scrollY), before.page);
  assert.equal(
    await page.locator("#nested-scroll").evaluate((node) => node.scrollTop),
    before.parent,
  );
  await menu.evaluate((node) => (node.scrollTop = node.scrollHeight));
  for (let i = 0; i < 3; i++) await page.mouse.wheel(0, 700);
  await page.waitForTimeout(200);
  assert.equal(
    await page.evaluate(() => scrollY),
    before.page,
    "bottom edge must not scroll the page",
  );
  assert.equal(
    await page.locator("#nested-scroll").evaluate((node) => node.scrollTop),
    before.parent,
    "bottom edge must not chain to a nested parent",
  );
  await menu.evaluate((node) => (node.scrollTop = 0));
  for (let i = 0; i < 3; i++) await page.mouse.wheel(0, -700);
  await page.waitForTimeout(200);
  assert.equal(await page.evaluate(() => scrollY), before.page);
  assert.equal(
    await page.locator("#nested-scroll").evaluate((node) => node.scrollTop),
    before.parent,
  );
  await page.mouse.move(1100, 700);
  await page.mouse.wheel(0, 500);
  await page.waitForTimeout(200);
  assert.equal(
    await page.evaluate(() => scrollY),
    before.page,
    "page stays locked even with pointer outside an open menu",
  );
  await single.press("Escape");
  assert.deepEqual(
    await page.evaluate(() => ({
      overflow: document.body.style.overflow,
      padding: document.body.style.paddingRight,
    })),
    original,
  );
  await page.mouse.wheel(0, 500);
  await page.waitForFunction(() => scrollY > 100);
  await page.evaluate(() => scrollTo(0, 0));
  await single.click();
  await page.locator("#outside").click();
  assert.equal(await single.getAttribute("aria-expanded"), "false");
  assert.equal(
    await page.evaluate(() => document.body.style.overflow),
    original.overflow,
  );
  await single.click();
  await page.evaluate(() => window.__selectQA.setDisabled(true));
  await page.waitForFunction(
    () => !document.querySelector(".custom-select-menu"),
  );
  assert.equal(
    await page.evaluate(() => document.body.style.overflow),
    original.overflow,
  );
  await page.evaluate(() => window.__selectQA.setDisabled(false));
  await single.click();
  await page.evaluate(() => window.__selectQA.setMounted(false));
  await page.waitForFunction(
    () => !document.querySelector(".custom-select-menu"),
  );
  assert.equal(
    await page.evaluate(() => document.body.style.overflow),
    original.overflow,
  );

  // These controls start below the fold. Selection must preserve their viewport
  // position even when the parent inserts/removes fields above the control.
  const position = (control) =>
    control.evaluate((node) => ({
      top: node.getBoundingClientRect().top,
      page: window.scrollY,
      parent: document.getElementById("dynamic-scroll").scrollTop,
    }));
  const assertPosition = async (
    control,
    before,
    message,
    sameScroll = false,
  ) => {
    await page.evaluate(
      () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(done)),
        ),
    );
    const after = await position(control);
    assert.ok(
      Math.abs(after.top - before.top) <= 1,
      `${message}: control moved from ${before.top} to ${after.top}; scroll ${before.page}/${before.parent} -> ${after.page}/${after.parent}`,
    );
    if (sameScroll) assert.equal(after.page, before.page, message);
  };
  const kind = page.getByRole("combobox", { name: "问题类型", exact: true });
  await page.locator("#dynamic-scroll").evaluate((node) => {
    node.scrollTop = 400;
    window.scrollTo(0, node.getBoundingClientRect().top + window.scrollY - 100);
  });
  for (const name of ["网络", "数据", "功能"]) {
    await kind.click();
    const before = await position(kind);
    await page.getByRole("option", { name, exact: true }).click();
    assert.equal(await kind.getAttribute("aria-expanded"), "false");
    await assertPosition(
      kind,
      before,
      `dynamic fields after selecting ${name}`,
    );
  }
  await kind.click();
  const keyboardBefore = await position(kind);
  await kind.press("ArrowDown");
  await kind.press("Enter");
  await assertPosition(
    kind,
    keyboardBefore,
    "keyboard selection with dynamic fields",
  );

  await page.evaluate(() => {
    window.__selectQA.setNested(false);
    window.__selectQA.setKind("功能");
  });
  await page.waitForFunction(
    () =>
      getComputedStyle(document.getElementById("dynamic-scroll")).overflowY ===
      "visible",
  );
  await kind.evaluate((node) =>
    window.scrollTo(0, node.getBoundingClientRect().top + window.scrollY - 200),
  );
  for (const name of ["网络", "数据", "功能"]) {
    await kind.click();
    const before = await position(kind);
    await page.getByRole("option", { name, exact: true }).click();
    await assertPosition(kind, before, `page fields after selecting ${name}`);
  }

  const pageSingle = page.getByRole("combobox", {
    name: "页面单选",
    exact: true,
  });
  const pageMulti = page.getByRole("combobox", {
    name: "页面多选",
    exact: true,
  });
  await pageSingle.evaluate((node) =>
    window.scrollTo(0, node.getBoundingClientRect().top + window.scrollY - 200),
  );
  await pageSingle.click();
  const singleBefore = await position(pageSingle);
  await page
    .getByRole("listbox")
    .evaluate((node) => (node.scrollTop = node.scrollHeight));
  await page.getByRole("option", { name: "选项 59", exact: true }).click();
  await assertPosition(
    pageSingle,
    singleBefore,
    "scrolled single selection",
    true,
  );
  assert.equal(
    await pageSingle.evaluate((node) => node === document.activeElement),
    true,
  );
  await pageMulti.click();
  const multiBefore = await position(pageMulti);
  await page
    .getByRole("listbox")
    .evaluate((node) => (node.scrollTop = node.scrollHeight));
  const menuBefore = await page
    .getByRole("listbox")
    .evaluate((node) => node.scrollTop);
  await page.getByRole("option", { name: "选项 59", exact: true }).click();
  await assertPosition(
    pageMulti,
    multiBefore,
    "scrolled multi selection",
    true,
  );
  assert.equal(await pageMulti.getAttribute("aria-expanded"), "true");
  assert.equal(
    await page.getByRole("listbox").evaluate((node) => node.scrollTop),
    menuBefore,
  );
  await pageMulti.press("Escape");
  assert.deepEqual(errors, []);
  const scenarios = [
    "single selection and reselect close inside form label",
    "multi selection and deselection remain open",
    "menu scrolls without moving page or nested parent",
    "bottom and top edges do not chain",
    "open menu locks scrolling outside the menu",
    "Escape restores styles and page scrolling",
    "outside click closes and unlocks",
    "disabled and unmounted selects release the scroll lock",
    "network/data fields preserve selection position in nested scrolling",
    "keyboard selection preserves position when fields appear",
    "network/data fields preserve selection position in page scrolling",
    "single selection below the fold preserves page position and focus",
    "multi selection below the fold preserves page and menu scroll",
  ];
  fs.writeFileSync(
    path.join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        browserEngine: engineName,
        scenarios,
        mode: "actual CustomSelect browser interactions; no IPC or remote requests",
      },
      null,
      2,
    ),
  );
  console.log(
    `${scenarios.length} custom select interaction scenarios passed.`,
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

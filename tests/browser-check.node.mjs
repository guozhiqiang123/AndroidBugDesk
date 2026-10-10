import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  recordBrowserFailure,
  startBrowserCheck,
} from "../scripts/browser-check-support.mjs";

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "browser check "));
  t.after(() => rm(root, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  await writeFile(join(root, ".gitignore"), "artifacts/\nnode_modules/\n");
  await writeFile(join(root, "index.html"), "<h1>fixture</h1>");
  return root;
}

test("simultaneous checks use different working ports and close their own servers", async (t) => {
  const root = await fixture(t);
  const a = await startBrowserCheck(join(root, "artifacts/a"), root);
  const b = await startBrowserCheck(join(root, "artifacts/b"), root);
  try {
    assert.notEqual(a.baseUrl, b.baseUrl);
    assert.match(await (await fetch(a.baseUrl)).text(), /fixture/);
    assert.match(await (await fetch(b.baseUrl)).text(), /fixture/);
    await a.verifySource();
    await b.verifySource();
  } finally {
    await Promise.all([a.close(), b.close()]);
  }
  await assert.rejects(fetch(a.baseUrl));
  await assert.rejects(fetch(b.baseUrl));
});

test("source changes invalidate a check; documentation edits do not", async (t) => {
  const root = await fixture(t);
  const server = await startBrowserCheck(join(root, "artifacts/check"), root);
  try {
    await writeFile(join(root, "README.md"), "documentation only");
    await server.verifySource();
    await writeFile(join(root, "index.html"), "changed source");
    await assert.rejects(server.verifySource(), /Source changed/);
  } finally {
    await server.close();
  }
});

test("failure replaces stale success and records screenshot, HTML and primary error", async (t) => {
  const root = await fixture(t);
  const output = join(root, "artifacts/failure");
  await mkdir(output, { recursive: true });
  await writeFile(join(output, "result.json"), '{"passed":true}');
  const page = {
    url: () => "http://127.0.0.1/mock",
    screenshot: ({ path }) => writeFile(path, "screenshot fixture"),
    content: async () => "<h1>failed state</h1>",
  };
  await recordBrowserFailure(output, new Error("detached control"), {
    contexts: () => [{ pages: () => [page] }],
  });
  const result = JSON.parse(
    await readFile(join(output, "result.json"), "utf8"),
  );
  assert.equal(result.passed, false);
  assert.match(result.error, /detached control/);
  assert.equal(
    await readFile(join(output, "failure-0.html"), "utf8"),
    "<h1>failed state</h1>",
  );
  assert.equal(
    await readFile(join(output, "failure-0.png"), "utf8"),
    "screenshot fixture",
  );
});

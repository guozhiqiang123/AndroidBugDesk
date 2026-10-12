import { mkdir, writeFile, appendFile, realpath } from "node:fs/promises";
import { resolve, join } from "node:path";
import { createServer, createLogger } from "vite";
import { sourceSnapshot, assertSourceUnchanged } from "./workflow-source.mjs";

export async function startBrowserCheck(output, projectDir = process.cwd()) {
  output = resolve(output);
  projectDir = await realpath(resolve(projectDir));
  await mkdir(output, { recursive: true });
  // Remove stale success before a new run. A failure must never retain passed:true.
  await writeFile(
    join(output, "result.json"),
    JSON.stringify({ passed: false, running: true }) + "\n",
  );
  const snapshot = await sourceSnapshot(projectDir, true);
  const log = join(output, "vite.log");
  await writeFile(log, "");
  let writes = Promise.resolve();
  const logger = createLogger();
  for (const method of ["info", "warn", "warnOnce", "error"]) {
    logger[method] = (message) => {
      writes = writes.then(() => appendFile(log, `${message}\n`));
    };
  }
  const started = performance.now();
  const server = await createServer({
    root: projectDir,
    customLogger: logger,
    server: { host: "127.0.0.1", port: 0, strictPort: true, open: false },
  });
  try {
    await server.listen();
  } catch (error) {
    await server.close();
    throw error;
  }
  const address = server.httpServer.address();
  const baseUrl = `http://127.0.0.1:${address.port}`;
  await writeFile(
    join(output, "run.json"),
    JSON.stringify({ baseUrl, sourceSnapshot: snapshot }, null, 2) + "\n",
  );
  return {
    baseUrl,
    verifySource: () => assertSourceUnchanged(projectDir, snapshot, true),
    async close() {
      await server.close();
      await writes;
      await writeFile(
        join(output, "timing.json"),
        JSON.stringify({
          durationMs: Math.round(performance.now() - started),
        }) + "\n",
      );
    },
  };
}

export async function recordBrowserFailure(output, error, browser) {
  const failure = {
    passed: false,
    error: String(error),
    stack: error?.stack,
    pages: [],
  };
  try {
    const pages =
      browser
        ?.contexts()
        .flatMap((context) => context.pages())
        .slice(-3) ?? [];
    for (const [index, page] of pages.entries()) {
      failure.pages.push({ url: page.url() });
      try {
        await page.screenshot({
          path: join(output, `failure-${index}.png`),
          timeout: 3000,
        });
        await writeFile(
          join(output, `failure-${index}.html`),
          await page.content(),
        );
      } catch (diagnosticError) {
        failure.pages[index].diagnosticError = String(diagnosticError);
      }
    }
  } finally {
    await writeFile(
      join(output, "result.json"),
      JSON.stringify(failure, null, 2) + "\n",
    );
  }
}

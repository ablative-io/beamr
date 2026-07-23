#!/usr/bin/env node
// FIX-BUNDLE-PACKAGER-NOARG harness: drives the packaged single-file
// bundle's NO-ARGUMENT init path by name in BOTH hosts. Never passes wasm
// bytes — that approximation is how the defect stayed latent.
//
//   node noarg-init-harness.mjs <path-to-beamr.bundle.mjs> <label>
//
// Writes noarg-node-<label>.txt and noarg-browser-<label>.txt beside
// itself. Exit code 0 iff BOTH legs resolved a VM.
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const bundlePath = resolve(process.argv[2] ?? "crates/beamr-wasm/pkg/beamr.bundle.mjs");
const label = process.argv[3] ?? "run";
const CHROME = process.env.CHROME_BIN
  ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

function record(leg, lines) {
  const text = lines.join("\n") + "\n";
  writeFileSync(join(HERE, `noarg-${leg}-${label}.txt`), text);
  console.log(`--- ${leg} ---\n${text}`);
}

// Leg 1 — Node host, in-process.
async function nodeLeg() {
  const lines = [`bundle: ${bundlePath}`, `node: ${process.version}`, "call: createPreloadedVm()  (no argument)"];
  try {
    const bundle = await import(pathToFileURL(bundlePath));
    const { vm, loads } = await bundle.createPreloadedVm();
    const ok = typeof vm?.spawn === "function";
    lines.push(`resolved: vm constructed, spawn surface ${ok ? "present" : "MISSING"}`);
    lines.push(`module loads: ${JSON.stringify(loads)}`);
    record("node", lines);
    return ok;
  } catch (error) {
    lines.push(`FAILED: ${error?.code ?? error?.name ?? "Error"}: ${error?.message ?? error}`);
    record("node", lines);
    return false;
  }
}

// Leg 2 — real browser context (headless Chrome) against a served page.
async function browserLeg() {
  const lines = [`bundle: ${bundlePath}`, `chrome: ${CHROME}`, "call: createPreloadedVm()  (no argument)"];
  let resolveResult;
  const result = new Promise((r) => { resolveResult = r; });
  const page = `<!DOCTYPE html><script type="module">
    const report = (payload) => fetch("/result", { method: "POST", body: JSON.stringify(payload) });
    try {
      const bundle = await import("/bundle.mjs");
      const { vm, loads } = await bundle.createPreloadedVm();
      await report({ ok: typeof vm?.spawn === "function", loads });
    } catch (error) {
      await report({ ok: false, failure: \`\${error?.name ?? "Error"}: \${error?.message ?? error}\` });
    }
  </script>`;
  const server = createServer((req, res) => {
    if (req.method === "POST" && req.url === "/result") {
      let body = "";
      req.on("data", (c) => { body += c; });
      req.on("end", () => { res.end("ok"); resolveResult(JSON.parse(body)); });
      return;
    }
    if (req.url === "/bundle.mjs") {
      res.setHeader("content-type", "text/javascript");
      return res.end(readFileSync(bundlePath));
    }
    res.setHeader("content-type", "text/html");
    res.end(page);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const chrome = spawn(CHROME, [
    "--headless=new", "--disable-gpu", "--remote-debugging-port=0",
    `--user-data-dir=${join(HERE, ".chrome-profile")}`, `${origin}/`,
  ], { stdio: "ignore" });
  const timeout = new Promise((r) => setTimeout(() => r({ ok: false, failure: "harness timeout (60s) — no report from page" }), 60_000));
  const outcome = await Promise.race([result, timeout]);
  chrome.kill();
  server.close();
  if (outcome.ok) {
    lines.push("resolved: vm constructed, spawn surface present");
    lines.push(`module loads: ${JSON.stringify(outcome.loads)}`);
  } else {
    lines.push(`FAILED: ${outcome.failure}`);
  }
  record("browser", lines);
  return outcome.ok;
}

const nodeOk = await nodeLeg();
const browserOk = await browserLeg();
console.log(`node: ${nodeOk ? "GREEN" : "RED"}  browser: ${browserOk ? "GREEN" : "RED"}`);
process.exit(nodeOk && browserOk ? 0 : 1);

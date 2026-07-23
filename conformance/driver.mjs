#!/usr/bin/env node
// WPORT-9 conformance driver (brief R3/R4/R5): one driverless harness,
// three real artifacts, machine-checked. Node stdlib only; the browser
// legs use the committed spawn+POST pattern (never CDP). Every
// prerequisite is FAIL-LOUD: a missing tool or browser errors by name —
// no leg can skip.
//
//   node conformance/driver.mjs
//
// Exit 0 iff ALL legs pass. The per-leg verdict names printed at the end
// are CI carriers (workflow moves them same-commit with any change).
import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, readFileSync, statSync } from "node:fs";
import { globSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  makeTracker,
  makeFetchCapability,
  makeKvCapability,
  runWorkload,
  runOutputLeg,
  runProcessErrorLeg,
} from "./page/workload-runner.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..");
const WORKLOAD_DIR = join(HERE, "workload");
const PAGE_DIR = join(HERE, "page");
const PKG_DIR = join(REPO_ROOT, "crates", "beamr-wasm", "pkg");
const FIXTURES_DIR = join(REPO_ROOT, "crates", "beamr-wasm", "fixtures");
const CHROME = process.env.CHROME_BIN
  ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const verdicts = [];
function verdict(name, ok, detail) {
  verdicts.push({ name, ok, detail });
  console.log(`leg ${ok ? "PASS" : "FAIL"}  ${name}${ok ? "" : `  — ${detail}`}`);
}

function run(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  });
  if (result.error?.code === "ENOENT") {
    throw new Error(`required tool missing: ${cmd} — install it; this gate never skips`);
  }
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} failed (exit ${result.status}):\n${result.stdout}\n${result.stderr}`);
  }
  return result.stdout;
}

// ---------------------------------------------------------------------------
// Phase 1 — BUILD: wasm-pack pkg (workload swept in) + single-file bundle.
// ---------------------------------------------------------------------------
function buildArtifacts() {
  console.log(`wasm-pack: ${run("wasm-pack", ["--version"]).trim()}`);
  console.log(`rustc: ${run("rustc", ["--version"]).trim()}`);
  console.log(`node: ${process.version}`);
  run("wasm-pack", ["build", "crates/beamr-wasm", "--target", "web", "--out-dir", "pkg"], {
    env: { ...process.env, BEAMR_WASM_BUNDLE_DIR: WORKLOAD_DIR },
    stdio: ["ignore", "inherit", "inherit"],
  });
  const bootstraps = globSync(
    join(REPO_ROOT, "target", "wasm32-unknown-unknown", "release", "build", "beamr-wasm-*", "out", "beamr-wasm-bundle", "bootstrap.js"),
  );
  if (bootstraps.length === 0) {
    throw new Error("no generated bootstrap.js found under the default target dir");
  }
  bootstraps.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  const bundleOut = dirname(bootstraps[0]);
  copyFileSync(bootstraps[0], join(PKG_DIR, "bootstrap.js"));
  run("node", [join(bundleOut, "package-bundle.mjs"), PKG_DIR]);
  const singleFile = join(PKG_DIR, "beamr.bundle.mjs");
  if (!existsSync(singleFile)) {
    throw new Error("package-bundle.mjs did not produce beamr.bundle.mjs");
  }
  return singleFile;
}

// ---------------------------------------------------------------------------
// Phase 2 — NODE LEG: packaged single-file bundle, NO-ARG init (never
// bytes — the lane harness's law, carried verbatim), full workload.
// ---------------------------------------------------------------------------
async function nodeLeg(singleFile) {
  const originals = { setTimeout: globalThis.setTimeout.bind(globalThis) };
  const bundle = await import(pathToFileURL(singleFile));
  const { vm } = await bundle.createPreloadedVm();
  if (typeof vm?.spawn !== "function") {
    throw new Error("no-arg init resolved no spawn surface");
  }
  const tracker = makeTracker(originals);
  const encoder = new TextEncoder();
  vm.register_fetch_capability(makeFetchCapability(async () => ({
    status: 200,
    headers: { "x-wport9-probe": "ok" },
    body: encoder.encode("wport9 probe ok (node scripted)"),
  }), tracker));
  vm.register_kv_capability(makeKvCapability(tracker));
  const env = { vm, tracker, originals, fetchUrl: "memory://wport9-probe" };
  const entries = await runWorkload(env);
  const output = await runOutputLeg(env);
  const processError = await runProcessErrorLeg(env);
  const all = [...entries, output, processError];
  const failed = all.filter((r) => !r.ok);
  return {
    ok: failed.length === 0,
    detail: failed.length === 0
      ? `${all.length} checks`
      : failed.map((f) => `${f.name}: ${f.detail}`).join("; "),
  };
}

// ---------------------------------------------------------------------------
// Phase 3 — loopback server for the browser legs.
// ---------------------------------------------------------------------------
const PAGE_HTML = (mode) => `<!DOCTYPE html>
<meta charset="utf-8">
<title>wport9 conformance — ${mode}</title>
<script>
  // F-0d shim — installs BEFORE any bundle byte evaluates (kit
  // precedent). The harness itself uses only saved originals, so every
  // recorded event is the bundle's.
  (() => {
    const originals = {
      setTimeout: window.setTimeout.bind(window),
      clearTimeout: window.clearTimeout.bind(window),
      setInterval: window.setInterval.bind(window),
      clearInterval: window.clearInterval.bind(window),
      requestAnimationFrame: window.requestAnimationFrame.bind(window),
    };
    const events = [];
    const record = (api, delay) => events.push({ api, delay_ms: delay ?? null, at_ms: performance.now() });
    window.setTimeout = (fn, delay, ...rest) => { record("setTimeout", delay); return originals.setTimeout(fn, delay, ...rest); };
    window.setInterval = (fn, delay, ...rest) => { record("setInterval", delay); return originals.setInterval(fn, delay, ...rest); };
    window.requestAnimationFrame = (fn) => { record("requestAnimationFrame", null); return originals.requestAnimationFrame(fn); };
    window.clearTimeout = (id) => { record("clearTimeout", null); return originals.clearTimeout(id); };
    window.clearInterval = (id) => { record("clearInterval", null); return originals.clearInterval(id); };
    window.__wport9 = { originals, timerShimEvents: events, mode: ${JSON.stringify(mode)} };
  })();
</script>
<body>
<script type="module" src="/page.mjs"></script>
</body>`;

const MANIFEST = JSON.stringify({
  format: "beamr-fetch-manifest",
  version: 1,
  modules: [
    { name: "fetch_chain_c", url: "fetch_chain_c.beam", deps: [] },
    { name: "fetch_chain_b", url: "fetch_chain_b.beam", deps: ["fetch_chain_c"] },
    { name: "fetch_chain_a", url: "fetch_chain_a.beam", deps: ["fetch_chain_b"] },
  ],
});

function startServer(onResult) {
  const server = createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (req.method === "POST" && url.pathname === "/result") {
      let body = "";
      req.on("data", (chunk) => { body += chunk; });
      req.on("end", () => { res.end("ok"); onResult(JSON.parse(body)); });
      return;
    }
    const respond = (type, payload) => {
      res.setHeader("content-type", type);
      res.end(payload);
    };
    if (url.pathname === "/bootstrap") return respond("text/html", PAGE_HTML("bootstrap"));
    if (url.pathname === "/single") return respond("text/html", PAGE_HTML("single"));
    if (url.pathname === "/page.mjs" || url.pathname === "/workload-runner.mjs" || url.pathname === "/worker-leg.mjs") {
      return respond("text/javascript", readFileSync(join(PAGE_DIR, url.pathname.slice(1))));
    }
    if (url.pathname === "/bundle.mjs") {
      return respond("text/javascript", readFileSync(join(PKG_DIR, "beamr.bundle.mjs")));
    }
    if (url.pathname.startsWith("/bundle/")) {
      const file = join(PKG_DIR, url.pathname.slice("/bundle/".length));
      const type = file.endsWith(".wasm") ? "application/wasm" : file.endsWith(".js") ? "text/javascript" : "application/octet-stream";
      return respond(type, readFileSync(file));
    }
    if (url.pathname === "/manifest/manifest.json") return respond("application/json", MANIFEST);
    if (url.pathname.startsWith("/manifest/")) {
      return respond("application/octet-stream", readFileSync(join(FIXTURES_DIR, url.pathname.slice("/manifest/".length))));
    }
    if (url.pathname === "/probe/ok") {
      res.setHeader("x-wport9-probe", "ok");
      return respond("text/plain", "wport9 probe ok (loopback)");
    }
    res.statusCode = 404;
    res.end("not found");
  });
  return new Promise((resolveStart) => {
    server.listen(0, "127.0.0.1", () => resolveStart(server));
  });
}

// ---------------------------------------------------------------------------
// Phase 4 — browser passes: headless Chrome, spawn+POST, fresh profile.
// ---------------------------------------------------------------------------
async function browserPass(pathName, expectedLegs, timeoutMs = 180000) {
  if (!existsSync(CHROME)) {
    throw new Error(`browser missing: ${CHROME} not found — set CHROME_BIN; this gate never skips`);
  }
  const received = new Map();
  let resolveDone;
  const done = new Promise((r) => { resolveDone = r; });
  const server = await startServer((message) => {
    if (message.done) return resolveDone();
    received.set(message.leg, message);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  const profile = join(HERE, `.chrome-profile-${pathName.slice(1)}`);
  const chrome = spawn(CHROME, [
    "--headless=new", "--disable-gpu", "--remote-debugging-port=0",
    `--user-data-dir=${profile}`, `${origin}${pathName}`,
  ], { stdio: "ignore" });
  const timeout = new Promise((r) => setTimeout(r, timeoutMs, "timeout"));
  const outcome = await Promise.race([done, timeout]);
  chrome.kill();
  server.close();
  if (outcome === "timeout") {
    throw new Error(`browser pass ${pathName}: no completion within ${timeoutMs}ms`);
  }
  for (const leg of expectedLegs) {
    const message = received.get(leg);
    if (!message) {
      verdict(leg, false, "leg never reported");
    } else {
      verdict(leg, message.ok, message.detail ?? "");
    }
  }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
let singleFile;
try {
  singleFile = buildArtifacts();
  verdict("build-artifacts", true, "pkg + bootstrap + beamr.bundle.mjs");
} catch (error) {
  verdict("build-artifacts", false, String(error.message ?? error));
}

if (verdicts.every((v) => v.ok)) {
  try {
    const node = await nodeLeg(singleFile);
    verdict("node-singlefile-noarg", node.ok, node.detail);
  } catch (error) {
    verdict("node-singlefile-noarg", false, String(error.message ?? error));
  }

  try {
    await browserPass("/bootstrap", [
      "browser-bootstrap-workload",
      "browser-bootstrap-loader",
      "browser-bootstrap-output",
      "browser-bootstrap-process-error",
      "browser-bootstrap-f0d-idle",
      "browser-bootstrap-f0d-armed",
      "browser-worker-workload",
      "browser-worker-f0d",
    ]);
  } catch (error) {
    verdict("browser-bootstrap-pass", false, String(error.message ?? error));
  }

  try {
    await browserPass("/single", ["browser-singlefile-noarg"]);
  } catch (error) {
    verdict("browser-singlefile-pass", false, String(error.message ?? error));
  }
}

console.log("\n=== WPORT-9 conformance verdicts ===");
for (const v of verdicts) {
  console.log(`${v.ok ? "PASS" : "FAIL"}  ${v.name}`);
}
const failed = verdicts.filter((v) => !v.ok);
console.log(`result: ${verdicts.length - failed.length}/${verdicts.length} legs passed`);
process.exit(failed.length === 0 ? 0 : 1);

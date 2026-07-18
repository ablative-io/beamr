// Three-pillar probe sitting — CDP driver (zero-dep, Node stdlib only).
// Pattern credit: Apollo Biscuit's haematite run.mjs (haematite ad4302e,
// crates/haematite-wasm-probes) — dependency-free Chrome automation, no chromedriver.
//
// Probes served: WPORT-3-PROBE-THROTTLE, WPORT-6-PROBE-FETCH, WPORT-7-PROBE-FAILURE
// (docs/design/beamr/probes/). Observations are evidence attached to those
// artifacts — NEVER CI acceptance. No assertion in this driver is a timing wall.
//
// Modes:
//   node driver.mjs <probe-page> --headless          (WPORT-6, WPORT-7 legs)
//   node driver.mjs <probe-page> --headed            (WPORT-3 throttle run (a): real
//     tab-backgrounding via Target.activateTarget + window minimize via
//     Browser.setWindowBounds; window appears on the operator's display)

import { spawn } from "node:child_process";
import { createReadStream, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WEB = join(HERE, "web");
const OBS = join(HERE, "observations");
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

function contentType(path) {
  switch (extname(path)) {
    case ".html": return "text/html; charset=utf-8";
    case ".js": case ".mjs": return "text/javascript; charset=utf-8";
    case ".wasm": return "application/wasm";
    case ".json": return "application/json; charset=utf-8";
    case ".beam": return "application/octet-stream";
    default: return "application/octet-stream";
  }
}

export function startServer(root = WEB) {
  const server = createServer((request, response) => {
    const requestPath = new URL(request.url, "http://127.0.0.1").pathname;
    const local = requestPath === "/" ? "index.html" : decodeURIComponent(requestPath.slice(1));
    const path = normalize(join(root, local));
    if (!path.startsWith(`${root}/`) || !existsSync(path) || !statSync(path).isFile()) {
      response.writeHead(404).end("not found");
      return;
    }
    response.writeHead(200, {
      "Content-Type": contentType(path),
      "Cache-Control": "no-store",
    });
    createReadStream(path).pipe(response);
    server.requestLog?.push({ t: Date.now(), path: requestPath });
  });
  server.requestLog = [];
  return new Promise((resolveServer, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      resolveServer({ server, origin: `http://127.0.0.1:${server.address().port}` });
    });
  });
}

export function launchChrome(profile, { headed = false } = {}) {
  const args = [
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--metrics-recording-only",
    "--password-store=basic",
    // Deliberately ABSENT: --disable-background-timer-throttling and friends.
    // Real platform throttling is exactly what WPORT-3 observes.
    "about:blank",
  ];
  if (!headed) args.unshift("--headless=new");
  const chrome = spawn(CHROME, args, { stdio: ["ignore", "pipe", "pipe"] });
  let pending = "";
  const endpoint = new Promise((resolveEndpoint, reject) => {
    chrome.once("error", reject);
    chrome.once("close", (code) => reject(new Error(`Chrome exited before CDP readiness (${code})`)));
    chrome.stderr.on("data", (chunk) => {
      pending += chunk.toString();
      const match = pending.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (match) resolveEndpoint(match[1]);
    });
  });
  return { chrome, endpoint };
}

export class CdpConnection {
  constructor(websocket) {
    this.websocket = websocket;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = [];
    websocket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(typeof data === "string" ? data : data.toString());
      if (message.id) {
        const waiter = this.pending.get(message.id);
        if (!waiter) return;
        this.pending.delete(message.id);
        if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
        else waiter.resolve(message.result);
        return;
      }
      for (const listener of this.listeners) listener(message);
    });
    websocket.addEventListener("close", () => {
      this.pending.forEach((waiter) => waiter.reject(new Error("CDP websocket closed")));
      this.pending.clear();
    });
  }
  onEvent(listener) { this.listeners.push(listener); }
  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    const message = { id, method, params };
    if (sessionId) message.sessionId = sessionId;
    return new Promise((resolveCommand, reject) => {
      this.pending.set(id, { resolve: resolveCommand, reject });
      this.websocket.send(JSON.stringify(message));
    });
  }
}

export function connectCdp(endpoint) {
  return new Promise((resolveConnection, reject) => {
    const websocket = new WebSocket(endpoint);
    websocket.addEventListener("open", () => resolveConnection(new CdpConnection(websocket)), { once: true });
    websocket.addEventListener("error", () => reject(new Error(`CDP websocket failed: ${endpoint}`)), { once: true });
  });
}

// Attach to a fresh page target; enable Runtime/Page/Network/Console; install the
// probeDriver binding the page calls with JSON payloads ({kind: "obs"|"suite-complete"|"suite-fatal"}).
export async function openProbePage(cdp, url, { onPayload, onNetwork, onConsole } = {}) {
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  await cdp.send("Runtime.enable", {}, sessionId);
  await cdp.send("Page.enable", {}, sessionId);
  await cdp.send("Network.enable", {}, sessionId);
  await cdp.send("Runtime.addBinding", { name: "probeDriver" }, sessionId);
  // Auto-attach so dedicated Workers surface as targets (WPORT-7 1b, WPORT-3 run (b)).
  await cdp.send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true }, sessionId);
  cdp.onEvent((event) => {
    if (event.sessionId !== sessionId) return;
    if (event.method === "Runtime.bindingCalled" && event.params.name === "probeDriver") {
      onPayload?.(JSON.parse(event.params.payload));
    } else if (event.method === "Network.requestWillBeSent") {
      onNetwork?.({ t: event.params.timestamp, url: event.params.request.url, id: event.params.requestId });
    } else if (event.method === "Runtime.consoleAPICalled") {
      onConsole?.({ t: event.params.timestamp, type: event.params.type, args: event.params.args.map(a => a.value ?? a.description) });
    } else if (event.method === "Runtime.exceptionThrown") {
      onConsole?.({ t: event.params.timestamp, type: "uncaught-exception", args: [event.params.exceptionDetails.text, event.params.exceptionDetails.exception?.description] });
    }
  });
  await cdp.send("Page.navigate", { url }, sessionId);
  return { targetId, sessionId };
}

// WPORT-3 run (a) helpers — REAL backgrounding, headed mode only.
export async function backgroundTarget(cdp, probeTargetId) {
  // Open a second tab and activate it: the probe tab becomes hidden (real
  // visibilitychange, real throttling eligibility). Then minimize the window.
  const { targetId: decoyId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  await cdp.send("Target.activateTarget", { targetId: decoyId });
  const { windowId } = await cdp.send("Browser.getWindowForTarget", { targetId: probeTargetId });
  await cdp.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "minimized" } });
  return { decoyId, windowId };
}

export async function foregroundTarget(cdp, probeTargetId, windowId) {
  await cdp.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
  await cdp.send("Target.activateTarget", { targetId: probeTargetId });
}

export function recordObservation(name, data) {
  mkdirSync(OBS, { recursive: true });
  const path = join(OBS, `${name}.json`);
  writeFileSync(path, JSON.stringify(data, null, 2));
  return path;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Shared runner orchestration over driver.mjs (zero-dep, Node stdlib only).
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  startServer, launchChrome, connectCdp, openProbePage,
} from "./driver.mjs";

// Boot a probe page: static server + Chrome + CDP + the probeDriver binding.
// Returns { cdp, origin, page, state, suite, close } where `suite` resolves on
// suite-complete and rejects on suite-fatal. `onObs(name, data)` fires per obs.
export async function bootProbe(pageName, { headed = false, onObs } = {}) {
  const { server, origin } = await startServer();
  const profile = mkdtempSync(join(tmpdir(), "probe-chrome-"));
  const { chrome, endpoint } = launchChrome(profile, { headed });
  const cdp = await connectCdp(await endpoint);

  const state = { obs: new Map(), network: [], console: [], suiteResult: null };
  let resolveSuite, rejectSuite;
  const suite = new Promise((res, rej) => { resolveSuite = res; rejectSuite = rej; });

  const page = await openProbePage(cdp, `${origin}/${pageName}`, {
    onPayload: (p) => {
      if (p.kind === "obs") { state.obs.set(p.name, p.data); onObs?.(p.name, p.data); }
      else if (p.kind === "suite-complete") { state.suiteResult = p; resolveSuite(p); }
      else if (p.kind === "suite-fatal") { rejectSuite(new Error(p.error)); }
    },
    onNetwork: (n) => state.network.push(n),
    onConsole: (c) => state.console.push(c),
  });

  const close = async () => {
    try { chrome.kill("SIGKILL"); } catch {}
    try { server.close(); } catch {}
    try { rmSync(profile, { recursive: true, force: true }); } catch {}
  };
  return { cdp, origin, page, state, suite, chrome, server, close };
}

// Reject a suite that never completes within `ms`.
export function withTimeout(promise, ms, label) {
  let timer;
  const guard = new Promise((_, rej) => { timer = setTimeout(() => rej(new Error(`${label} timed out after ${ms}ms`)), ms); });
  return Promise.race([promise.finally(() => clearTimeout(timer)), guard]);
}

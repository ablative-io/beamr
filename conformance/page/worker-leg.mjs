// WPORT-9 dedicated-Worker leg (brief R4, D3 reading i): the same real
// bundle + committed workload, in a module Worker, with the F-0d shim
// over the WORKER'S OWN timer globals installed before any bundle byte
// evaluates. Results post back to the page, which relays to the driver.
//
// The static runner import is safe ahead of the shim: workload-runner
// touches no timers at module-evaluation time. The bundle import is
// dynamic and strictly after the shim installs.
import {
  makeTracker,
  makeFetchCapability,
  makeKvCapability,
  runWorkload,
} from "/workload-runner.mjs";

const originals = {
  setTimeout: self.setTimeout.bind(self),
  clearTimeout: self.clearTimeout.bind(self),
  setInterval: self.setInterval.bind(self),
  clearInterval: self.clearInterval.bind(self),
};
const timerShimEvents = [];
const record = (api, delay) =>
  timerShimEvents.push({ api, delay_ms: delay ?? null, at_ms: performance.now() });
self.setTimeout = (fn, delay, ...rest) => { record("setTimeout", delay); return originals.setTimeout(fn, delay, ...rest); };
self.setInterval = (fn, delay, ...rest) => { record("setInterval", delay); return originals.setInterval(fn, delay, ...rest); };
self.clearTimeout = (id) => { record("clearTimeout", null); return originals.clearTimeout(id); };
self.clearInterval = (id) => { record("clearInterval", null); return originals.clearInterval(id); };

function browserFetchImpl() {
  return async (requestObject, signal) => {
    const response = await fetch(requestObject.url, { signal });
    const headers = {};
    response.headers.forEach((value, key) => { headers[key] = value; });
    return {
      status: response.status,
      headers,
      body: new Uint8Array(await response.arrayBuffer()),
    };
  };
}

const results = [];
try {
  const bundle = await import("/bundle/bootstrap.js");
  const { vm } = await bundle.createPreloadedVm();
  if (typeof vm?.spawn !== "function") {
    throw new Error("createPreloadedVm() resolved no spawn surface in the Worker");
  }
  const tracker = makeTracker(originals);
  vm.register_fetch_capability(makeFetchCapability(browserFetchImpl(), tracker));
  vm.register_kv_capability(makeKvCapability(tracker));
  const env = {
    vm,
    tracker,
    originals,
    fetchUrl: new URL("/probe/ok", self.location.href).href,
  };

  const workload = await runWorkload(env);
  const failed = workload.filter((r) => !r.ok);
  results.push({
    leg: "browser-worker-workload",
    ok: failed.length === 0,
    detail: failed.length === 0
      ? `${workload.length} checks`
      : failed.map((f) => `${f.name}: ${f.detail}`).join("; "),
  });

  // Worker F-0d window: true idle after the workload settles.
  const baseline = timerShimEvents.length;
  await new Promise((resolve) => originals.setTimeout(resolve, 600));
  const fresh = timerShimEvents.slice(baseline);
  results.push({
    leg: "browser-worker-f0d",
    ok: fresh.length === 0,
    detail: fresh.length === 0
      ? "zero events across 600ms worker window"
      : `events in window: ${JSON.stringify(fresh)}`,
  });
} catch (error) {
  results.push({ leg: "browser-worker-workload", ok: false, detail: String(error?.message ?? error) });
  results.push({ leg: "browser-worker-f0d", ok: false, detail: "not reached" });
}
self.postMessage(results);

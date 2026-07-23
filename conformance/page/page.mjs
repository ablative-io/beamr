// WPORT-9 browser page driver — runs in the F-0d-shimmed page the
// conformance driver serves. Mode "bootstrap": wasm-pack pkg + generated
// bootstrap, plus loader/output/process-error legs, the Worker leg, and
// the two F-0d windows. Mode "single": the packaged single-file bundle's
// NO-ARG init path (never bytes — the lane harness's law).
import {
  makeTracker,
  makeFetchCapability,
  makeKvCapability,
  runWorkload,
  runOutputLeg,
  runProcessErrorLeg,
} from "/workload-runner.mjs";

const { originals, timerShimEvents, mode } = window.__wport9;

async function post(leg, ok, detail) {
  await fetch("/result", { method: "POST", body: JSON.stringify({ leg, ok, detail }) });
}

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

function summarize(results) {
  const failed = results.filter((r) => !r.ok);
  return {
    ok: failed.length === 0,
    detail: failed.length === 0
      ? `${results.length} checks`
      : failed.map((f) => `${f.name}: ${f.detail}`).join("; "),
  };
}

async function makeVm(bundle) {
  const { vm } = await bundle.createPreloadedVm();
  if (typeof vm?.spawn !== "function") {
    throw new Error("createPreloadedVm() resolved no spawn surface");
  }
  const tracker = makeTracker(originals);
  vm.register_fetch_capability(makeFetchCapability(browserFetchImpl(), tracker));
  vm.register_kv_capability(makeKvCapability(tracker));
  return { vm, tracker, originals, fetchUrl: new URL("/probe/ok", location.href).href };
}

async function loaderLeg(env) {
  const manifestUrl = new URL("/manifest/manifest.json", location.href).href;
  const fetchFn = (url) => fetch(url).then((response) => {
    if (!response.ok) throw new Error(`http ${response.status} for ${url}`);
    return response.arrayBuffer();
  });
  const report = JSON.parse(await env.vm.load_artifacts(manifestUrl, fetchFn));
  const loadedNames = (report.loaded ?? []).map((entry) => entry.module);
  const expected = ["fetch_chain_c", "fetch_chain_b", "fetch_chain_a"];
  const ok = report.ok === true && JSON.stringify(loadedNames) === JSON.stringify(expected);
  return { ok, detail: ok ? `loaded ${loadedNames.join(",")}` : JSON.stringify(report) };
}

// F-0d window: zero bundle-attributable shim events across a
// settlement-quiet window bounded by an ORIGINAL timer. No arm-count or
// promptness claims (EARLY-UNDER-CACHED-CLOCK).
async function f0dWindow(windowMs) {
  const baseline = timerShimEvents.length;
  await new Promise((resolve) => originals.setTimeout(resolve, windowMs));
  const fresh = timerShimEvents.slice(baseline);
  return {
    ok: fresh.length === 0,
    detail: fresh.length === 0
      ? `zero events across ${windowMs}ms window`
      : `events in window: ${JSON.stringify(fresh)}`,
  };
}

async function workerLeg() {
  return new Promise((resolveLeg) => {
    const worker = new Worker("/worker-leg.mjs", { type: "module" });
    const timeout = originals.setTimeout(() => {
      worker.terminate();
      resolveLeg([
        { leg: "browser-worker-workload", ok: false, detail: "worker timeout" },
        { leg: "browser-worker-f0d", ok: false, detail: "worker timeout" },
      ]);
    }, 120000);
    worker.onmessage = (event) => {
      originals.clearTimeout(timeout);
      worker.terminate();
      resolveLeg(event.data);
    };
    worker.onerror = (event) => {
      originals.clearTimeout(timeout);
      worker.terminate();
      resolveLeg([
        { leg: "browser-worker-workload", ok: false, detail: `worker error: ${event.message}` },
        { leg: "browser-worker-f0d", ok: false, detail: `worker error: ${event.message}` },
      ]);
    };
  });
}

try {
  if (mode === "bootstrap") {
    const bundle = await import("/bundle/bootstrap.js");
    const env = await makeVm(bundle);

    const workload = summarize(await runWorkload(env));
    await post("browser-bootstrap-workload", workload.ok, workload.detail);

    const loader = await loaderLeg(env);
    await post("browser-bootstrap-loader", loader.ok, loader.detail);

    const output = await runOutputLeg(env);
    await post("browser-bootstrap-output", output.ok, JSON.stringify(output.detail));

    const processError = await runProcessErrorLeg(env);
    await post("browser-bootstrap-process-error", processError.ok, JSON.stringify(processError.detail));

    for (const message of await workerLeg()) {
      await post(message.leg, message.ok, message.detail);
    }

    // True idle: everything above has settled; nothing is armed.
    const idle = await f0dWindow(600);
    await post("browser-bootstrap-f0d-idle", idle.ok, idle.detail);

    // Armed future deadline: one far-future receive timer, then silence.
    // The arming one-shot lands before the baseline; the window must be
    // event-free (recurring callbacks while only a future deadline
    // exists are exactly what the gate forbids).
    env.vm.spawn("wport9_conformance", "armed_hold", "[]");
    await new Promise((resolve) => originals.setTimeout(resolve, 50));
    const armed = await f0dWindow(600);
    await post("browser-bootstrap-f0d-armed", armed.ok, armed.detail);
  } else {
    const bundle = await import("/bundle.mjs");
    const env = await makeVm(bundle);
    const workload = summarize(await runWorkload(env));
    await post("browser-singlefile-noarg", workload.ok, workload.detail);
  }
} catch (error) {
  await post(`browser-${mode}-fatal`, false, String(error?.message ?? error));
} finally {
  await fetch("/result", { method: "POST", body: JSON.stringify({ done: true }) });
}

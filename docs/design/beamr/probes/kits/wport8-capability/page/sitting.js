// WPORT-8 capability sitting driver. Runs the three protocol legs against
// the REAL generated bundle and writes each leg's evidence JSON to the kit
// server. The harness uses ONLY the saved timer originals (F-0d pattern) so
// the shim log records the bundle's host traffic, not the harness's own.
const { originals, timerShimEvents } = window.__sitting;
// The real generated bundle: wasm-pack pkg + generated bootstrap.js (the
// WPORT-6 sitting shape; the single-file packaging is bypassed — its
// no-arg init path is broken, flagged 2026-07-24 for its own lane).
const { createPreloadedVm, awaitExit } = await import("/bundle/bootstrap.js");

const HANDLER_MODULE = "wport8_probe";
const LEG_TIMEOUT_MS = 90_000;
const KV_DB_NAME = "wport8-sitting";
const KV_STORE = "kv";

const environment = {
  user_agent: navigator.userAgent,
  platform: navigator.platform,
  page_url: location.href,
  captured_at_iso: new Date().toISOString(),
  ...(await (await fetch("/sitting-env.json")).json()),
};

// --- capability objects, the edge-worker shape verbatim -------------------
// (examples/edge-worker/src/worker.js fetchCapability/kvCapability; the
// tracker wrap and the controller registry are HOST policy — invisible to
// the VM, which sees exactly the worker's object contract.)

function headersToObject(headers) {
  const object = Object.create(null);
  for (const [name, value] of headers) {
    object[name] = value;
  }
  return object;
}

function fetchCapability(controllers, tracker) {
  return {
    request(request, slot) {
      const controller = new AbortController();
      controllers.push(controller);
      // The bridge fires this hook when the calling BEAM process dies with
      // the request still in flight (process-death auto-abort).
      slot.abort = () => controller.abort();
      const init = {
        method: request.method || "GET",
        headers: request.headers || {},
        signal: controller.signal,
      };
      if (request.body != null && request.body !== "") {
        init.body = request.body;
      }
      return tracker.wrap(fetch(request.url, init).then(async (response) => ({
        status: response.status,
        headers: headersToObject(response.headers),
        body: await response.text(),
      })));
    },
  };
}

// IndexedDB-backed KV namespace exposing the Workers-KV-shaped surface the
// edge-worker's kvCapability wraps. String keys; IDB string key order is
// code-unit order = the contract's lexicographic listing.
async function openKvNamespace() {
  const db = await new Promise((resolve, reject) => {
    const open = indexedDB.open(KV_DB_NAME, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(KV_STORE);
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
  const run = (mode, operate) =>
    new Promise((resolve, reject) => {
      const store = db.transaction(KV_STORE, mode).objectStore(KV_STORE);
      const request = operate(store);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  return {
    get: async (key) => {
      const value = await run("readonly", (store) => store.get(key));
      return value === undefined ? undefined : value;
    },
    put: (key, value) => run("readwrite", (store) => store.put(value, key)),
    delete: (key) => run("readwrite", (store) => store.delete(key)),
    list: async ({ prefix }) => {
      const range = prefix === ""
        ? undefined
        : IDBKeyRange.bound(prefix, `${prefix}￿`);
      const keys = await run("readonly", (store) => store.getAllKeys(range));
      return { keys: keys.map((name) => ({ name })) };
    },
    clear: () => run("readwrite", (store) => store.clear()),
    dump: async () => {
      const keys = await run("readonly", (store) => store.getAllKeys());
      const contents = {};
      for (const key of keys) {
        contents[key] = await run("readonly", (store) => store.get(key));
      }
      return contents;
    },
  };
}

function kvCapability(namespace, tracker) {
  return {
    get: (key) => tracker.wrap(namespace.get(key)),
    put: (key, value) => tracker.wrap(namespace.put(key, value)),
    delete: (key) => tracker.wrap(namespace.delete(key)),
    list_by_prefix: (prefix) => tracker.wrap(
      namespace.list({ prefix }).then((listed) => listed.keys.map((entry) => entry.name)),
    ),
  };
}

// --- settlement-event drive loop ------------------------------------------
// Settled-idle contract (WPORT-2): await_exit registered while the process
// is parked on a capability op resolves "idle". Each idle resolution waits
// for the NEXT capability-promise settlement (plus one original-setTimeout
// macrotask so the arbiter's completion turn has run) before probing again
// — every probe is woken by a settlement event, never by a timer. The
// harness watchdog uses the saved original, so the shim log stays clean.
function makeTracker() {
  let settleWaiters = [];
  const announce = () => {
    const ready = settleWaiters;
    settleWaiters = [];
    ready.forEach((resolve) => resolve());
  };
  return {
    wrap(promise) {
      const done = () => originals.setTimeout(announce, 0);
      promise.then(done, done);
      return promise;
    },
    nextSettlement() {
      return new Promise((resolve) => settleWaiters.push(resolve));
    },
  };
}

async function settledExit(vm, pid, tracker, legName) {
  let timedOut = false;
  const watchdog = originals.setTimeout(() => { timedOut = true; }, LEG_TIMEOUT_MS);
  try {
    for (;;) {
      const exit = await awaitExit(vm, pid);
      if (exit.state !== "idle") return exit;
      if (timedOut) {
        throw new Error(`${legName}: still parked after ${LEG_TIMEOUT_MS}ms — recorded as a timeout, not a settle`);
      }
      await tracker.nextSettlement();
    }
  } finally {
    originals.clearTimeout(watchdog);
  }
}

// --- evidence --------------------------------------------------------------

async function writeEvidence(name, payload) {
  const body = JSON.stringify(
    { probe: "WPORT-8-PROBE-CAPABILITY", environment, ...payload },
    null,
    2,
  );
  const response = await fetch(`/evidence/${name}`, { method: "POST", body });
  if (!response.ok) {
    throw new Error(`evidence write failed for ${name}: ${response.status}`);
  }
}

const logElement = document.getElementById("log");
function log(line) {
  logElement.textContent += `${line}\n`;
}

function setStatus(id, cls, suffix) {
  const item = document.getElementById(id);
  item.className = cls;
  if (suffix) item.textContent = `${item.textContent.split(" — [")[0]} — [${suffix}]`;
}

async function runLeg(id, name, body) {
  setStatus(id, "running");
  try {
    await body();
    setStatus(id, "recorded", "evidence written");
  } catch (error) {
    setStatus(id, "failed", String(error));
    await writeEvidence(`${name}-FAILED`, { leg: name, failure: String(error) });
    throw error;
  }
}

// --- the legs --------------------------------------------------------------

async function leg1() {
  const namespace = await openKvNamespace();
  // Host policy, not contract: a cleared namespace makes the leg-1 listing
  // deterministic; the pre-seeded key proves list_by_prefix sees more than
  // the leg's own write.
  await namespace.clear();
  await namespace.put("wport8:preexisting", "seeded-before-spawn");

  const tracker = makeTracker();
  const controllers = [];
  const { vm, loads } = await createPreloadedVm();
  vm.register_fetch_capability(fetchCapability(controllers, tracker));
  vm.register_kv_capability(kvCapability(namespace, tracker));

  const spawnArgs = [`${location.origin}/probe/ok`, "wport8:sitting", "browser-idb-value", "wport8:"];
  const pid = vm.spawn(HANDLER_MODULE, "leg1", JSON.stringify(spawnArgs));
  const exit = await settledExit(vm, pid, tracker, "leg1");
  const idbContentsAfterSettle = await namespace.dump();

  log(`leg1 exit: ${JSON.stringify(exit.result)}`);
  await writeEvidence("wport8-leg1-end-to-end", {
    leg: "leg1 worker-shaped end-to-end (A4 rider 2)",
    bundle_module_loads: loads,
    spawn_args: spawnArgs,
    exit_envelope: exit,
    idb_contents_after_settle: idbContentsAfterSettle,
    manual_observation_remaining:
      "network log: exactly one /probe/ok request for this leg (README checklist)",
  });
}

async function leg2Reject() {
  const tracker = makeTracker();
  const { vm } = await createPreloadedVm();
  vm.register_fetch_capability(fetchCapability([], tracker));
  // A closed loopback port gives a deterministic connection failure with
  // the browser's real failure detail, without external-network dependence
  // (a truly unroutable IP can hang for minutes — wrong for a sitting).
  const target = "http://127.0.0.1:9/";
  const pid = vm.spawn(HANDLER_MODULE, "leg2_reject", JSON.stringify([target]));
  const exit = await settledExit(vm, pid, tracker, "leg2_reject");
  log(`leg2_reject exit: ${JSON.stringify(exit.result)}`);
  await writeEvidence("wport8-leg2-rejected", {
    leg: "leg2 rejection (refused-connection target)",
    target_url: target,
    exit_envelope: exit,
  });
}

async function leg2Cancel() {
  const tracker = makeTracker();
  const controllers = [];
  const { vm } = await createPreloadedVm();
  vm.register_fetch_capability(fetchCapability(controllers, tracker));
  const target = `${location.origin}/probe/slow?ms=30000`;
  const abortDelayMs = 400;
  const pid = vm.spawn(HANDLER_MODULE, "leg2_cancel", JSON.stringify([target]));
  originals.setTimeout(() => controllers.forEach((c) => c.abort()), abortDelayMs);
  const exit = await settledExit(vm, pid, tracker, "leg2_cancel");
  log(`leg2_cancel exit: ${JSON.stringify(exit.result)}`);
  await writeEvidence("wport8-leg2-cancelled-host-abort", {
    leg: "leg2 cancellation, host-abort arm (per the sitting-scope truing note)",
    target_url: target,
    host_abort_delay_ms: abortDelayMs,
    exit_envelope: exit,
    manual_observation_remaining:
      "network log: the 30s /probe/slow request shows (canceled) (README checklist)",
  });
}

async function leg2Refusal() {
  const tracker = makeTracker();
  const { vm } = await createPreloadedVm();
  // Deliberately NO kv capability on this VM; fetch registered so the
  // refusal is provably per-module, not per-VM.
  vm.register_fetch_capability(fetchCapability([], tracker));
  const pid = vm.spawn(HANDLER_MODULE, "leg2_refusal", JSON.stringify(["any-key"]));
  const exit = await settledExit(vm, pid, tracker, "leg2_refusal");
  log(`leg2_refusal exit: ${JSON.stringify(exit.result)}`);
  await writeEvidence("wport8-leg2-refusal-uninjected-kv", {
    leg: "leg2 refusal (VM with fetch but no kv capability)",
    exit_envelope: exit,
    refusal_is_synchronous:
      "the exit is retained before any capability promise exists; no settlement event was needed to reach it",
  });
}

async function leg3() {
  const tracker = makeTracker();
  const { vm } = await createPreloadedVm();
  vm.register_fetch_capability(fetchCapability([], tracker));
  const target = `${location.origin}/probe/slow?ms=1500`;
  const shimIndexAtSpawn = timerShimEvents.length;
  const spawnAtMs = performance.now();
  const pid = vm.spawn(HANDLER_MODULE, "leg3", JSON.stringify([target]));
  const exit = await settledExit(vm, pid, tracker, "leg3");
  const settleAtMs = performance.now();
  const windowEvents = timerShimEvents.slice(shimIndexAtSpawn);
  log(`leg3 exit: ${JSON.stringify(exit.result)}`);
  log(`leg3 shim events in window: ${JSON.stringify(windowEvents)}`);
  await writeEvidence("wport8-leg3-no-polling-timer-shim", {
    leg: "leg3 NO-POLLING under real completion (F-0d shim record)",
    target_url: target,
    in_flight_window: { spawn_at_ms: spawnAtMs, settle_observed_at_ms: settleAtMs },
    timer_shim_events_in_window: windowEvents,
    set_interval_calls_in_window: windowEvents.filter((e) => e.api === "setInterval").length,
    request_animation_frame_calls_in_window:
      windowEvents.filter((e) => e.api === "requestAnimationFrame").length,
    set_timeout_arms_in_window: windowEvents.filter((e) => e.api === "setTimeout").length,
    harness_timer_use_disclosure:
      "the harness itself uses only saved pre-shim originals (watchdog, settlement announce), so every event above was armed by the bundle or the platform",
    exit_envelope: exit,
  });
}

// --- suite ------------------------------------------------------------------

document.getElementById("run").disabled = false;
document.getElementById("run").addEventListener("click", async () => {
  document.getElementById("run").disabled = true;
  const startedAtIso = new Date().toISOString();
  const outcomes = {};
  const legs = [
    ["leg1", "leg1", leg1],
    ["leg2-reject", "leg2_reject", leg2Reject],
    ["leg2-cancel", "leg2_cancel", leg2Cancel],
    ["leg2-refusal", "leg2_refusal", leg2Refusal],
    ["leg3", "leg3", leg3],
  ];
  for (const [id, name, body] of legs) {
    try {
      await runLeg(id, name, body);
      outcomes[name] = "recorded";
    } catch (error) {
      outcomes[name] = `failed: ${error}`;
    }
  }
  await writeEvidence("wport8-suite", {
    suite: "WPORT-8 capability sitting — all legs",
    started_at_iso: startedAtIso,
    finished_at_iso: new Date().toISOString(),
    leg_outcomes: outcomes,
    timer_shim_events_total: timerShimEvents.length,
  });
  log("suite evidence written — run `node collect.mjs` (command 3) to stage it");
});
log("kit ready — bundle imported, handler module in the bundle manifest");
// Smoke convenience (disclosed in the README status section): ?autorun=1
// presses the button on load, so a headless-browser smoke can run the
// suite unattended. The official sitting uses the button.
if (new URLSearchParams(location.search).get("autorun") === "1") {
  document.getElementById("run").click();
}

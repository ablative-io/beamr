// Shared probe-page JS for the three beamr wasm probes (WPORT-6/7/3).
// Zero dependencies; ES module. Talks to the CDP driver EXCLUSIVELY through
// window.probeDriver(JSON.stringify(payload)).
//
// Payload kinds: {kind:"obs", name, data}, {kind:"suite-complete", ...},
// {kind:"suite-fatal", error}.

export const now = () => (typeof performance !== "undefined" ? performance.now() : Date.now());
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The one observation channel for the unified deadline service (WPORT-7 §116-119):
// UnifiedDeadlineSnapshot freight is cfg(test)-only, so setTimeout/clearTimeout
// spies ARE how host timer activity is observed on the shipped bundle. Install
// BEFORE constructing any WasmVm (HostPrimitives::probe captures globalThis
// setTimeout/clearTimeout at construction, so the VM captures these wrappers).
export function installTimerSpies(scope = globalThis) {
  const realSet = scope.setTimeout.bind(scope);
  const realClear = scope.clearTimeout.bind(scope);
  const record = {
    arms: [],
    clears: [],
    get armCount() { return record.arms.length; },
    get clearCount() { return record.clears.length; },
  };
  scope.setTimeout = function (handler, delay, ...rest) {
    const handle = realSet(handler, delay, ...rest);
    record.arms.push({ t: now(), delay: Number(delay) || 0, handle: String(handle) });
    return handle;
  };
  scope.clearTimeout = function (handle) {
    record.clears.push({ t: now(), handle: String(handle) });
    return realClear(handle);
  };
  return record;
}

// A compact spy summary suitable for embedding in an observation. `sinceT`
// optionally windows the counts to arms/clears at or after a timestamp.
export function spySummary(spy, sinceT = 0) {
  const arms = spy.arms.filter((a) => a.t >= sinceT);
  const clears = spy.clears.filter((c) => c.t >= sinceT);
  // "deadline-scale" arms are the unified one-shot (large delay); "macrotask"
  // arms are the arbiter fairness yield (delay 0) and page/self timers.
  const deadlineArms = arms.filter((a) => a.delay >= 1000);
  const zeroArms = arms.filter((a) => a.delay === 0);
  return {
    armCount: arms.length,
    clearCount: clears.length,
    deadlineScaleArmCount: deadlineArms.length,
    zeroDelayArmCount: zeroArms.length,
    deadlineArmDelays: deadlineArms.map((a) => a.delay),
    arms,
    clears,
  };
}

// Load the generated web bundle. `mod.default()` uses beamr_wasm.js's own
// import.meta.url to locate beamr_wasm_bg.wasm (same pkg/ dir), fetched over
// real HTTP from this origin.
export async function loadBundle(pkgUrl = "./pkg/beamr_wasm.js") {
  const mod = await import(pkgUrl);
  await mod.default();
  return mod;
}

// The one-line browser fetch adapter the WPORT-6 loader is fed (verbatim from
// WPORT-6-PROBE-FETCH.md): throws on !r.ok so a bad URL becomes an
// artifact_fetch_failed rejection.
export const fetchAdapter = (url) =>
  fetch(url).then((r) => {
    if (!r.ok) throw new Error(String(r.status));
    return r.arrayBuffer();
  });

// Fetch a .beam artifact as bytes for load_module (WPORT-7/3 workloads).
export async function fetchBeam(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`fetch ${url}: ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
}

// The WPORT-7 §2 strand measurement, IDENTICAL across environments (browser
// main / Worker / Node). `beamr` is the bundle namespace (has create_vm),
// `spy` is an installTimerSpies() record installed BEFORE any VM construction,
// `strandBeam` is the strand_probe.beam bytes. Records mid-turn send_after(0)
// delivery latency + the setTimeout/clearTimeout arms observed during each
// wait (the cfg(test) UnifiedDeadlineSnapshot freight is unavailable on the
// shipped bundle, so the spy is the counter channel).
export async function runStrandSamples({ beamr, spy, strandBeam, N = 20, environment }) {
  const samples = [];
  for (let i = 0; i < N; i++) {
    const vm = beamr.create_vm();
    vm.load_module(strandBeam);
    const pid = vm.spawn("strand_probe", "run", "[]");
    // Vary phase relative to host turn starts: a cycling number of macrotask
    // hops before the measured arm.
    for (let k = 0; k < (i % 4); k++) await new Promise((r) => setTimeout(r, 0));
    const armBefore = spy.arms.length;
    const clearBefore = spy.clears.length;
    const t0 = now();
    vm.send_message(pid, "go");
    const completion = JSON.parse(await vm.await_exit(pid));
    const t1 = now();
    const newArms = spy.arms.slice(armBefore);
    samples.push({
      i, latencyMs: +(t1 - t0).toFixed(3),
      state: completion.state, result: completion.result,
      armsDuringWait: newArms.length,
      clearsDuringWait: spy.clears.length - clearBefore,
      armDelaysDuringWait: newArms.map((a) => a.delay),
    });
  }
  const lat = samples.map((s) => s.latencyMs).sort((a, b) => a - b);
  const pct = (p) => lat[Math.min(lat.length - 1, Math.floor(p * lat.length))];
  return {
    environment,
    samples,
    latency: { n: lat.length, min: lat[0], p50: pct(0.5), p90: pct(0.9), max: lat[lat.length - 1],
      mean: +(lat.reduce((a, b) => a + b, 0) / lat.length).toFixed(3) },
    allExitedGotProbe: samples.every((s) => s.state === "exited" && s.result === "got_probe"),
    maxArmsDuringWait: Math.max(...samples.map((s) => s.armsDuringWait)),
    note: "CONFIRM if latency clusters near ~1000ms with arm churn per macrotask; KILL if macrotask-scale with one arm -> one fire.",
  };
}

export function obs(name, data) {
  window.probeDriver(JSON.stringify({ kind: "obs", name, data }));
}
export function suiteComplete(data = {}) {
  window.probeDriver(JSON.stringify({ kind: "suite-complete", ...data }));
}
export function suiteFatal(error) {
  window.probeDriver(
    JSON.stringify({ kind: "suite-fatal", error: String((error && error.stack) || error) })
  );
}

// A rolling window.onerror recorder (WPORT-7 1a/1c uncaught-trap channel).
// Installs once; each leg drains what it needs by timestamp.
export function installWindowOnerrorRecorder() {
  const events = [];
  const prior = window.onerror;
  window.onerror = function (message, source, lineno, colno, error) {
    events.push({
      t: now(),
      message: String(message),
      errorName: error && error.constructor ? error.constructor.name : null,
    });
    if (typeof prior === "function") return prior.apply(this, arguments);
    return false; // do not swallow; let devtools also see it
  };
  window.addEventListener("unhandledrejection", (e) => {
    events.push({ t: now(), message: "unhandledrejection: " + String(e.reason), errorName: "PromiseRejection" });
  });
  return events;
}

// Run one guarded call and classify: {returned} | {trapped, errorName, message}.
export function guardedCall(fn) {
  try {
    return { returned: fn() };
  } catch (e) {
    return { trapped: true, errorName: e && e.constructor ? e.constructor.name : null, message: String((e && e.message) || e).slice(0, 200) };
  }
}

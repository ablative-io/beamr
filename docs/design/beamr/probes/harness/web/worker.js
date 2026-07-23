// WPORT-7 leg 1b Worker + WPORT-3 run (b) Worker workload host.
// Runs a beamr VM inside a dedicated module Worker and reports back by
// postMessage (the owner page forwards to window.probeDriver). A panicking
// workload's trap surfaces to the WORKER's self.onerror (and propagates to the
// owner page's worker.onerror); the registered panic callback must run first.

import { loadBundle, fetchBeam, installTimerSpies, now, runStrandSamples } from "./probe-common.js";

const spy = installTimerSpies(self);
let beamr = null;

self.onerror = function (message, source, lineno, colno, error) {
  self.postMessage({ type: "self-onerror", t: now(), message: String(message),
    errorName: error && error.constructor ? error.constructor.name : null });
  return false; // allow propagation to the owner page's worker.onerror
};
self.addEventListener("unhandledrejection", (e) =>
  self.postMessage({ type: "self-unhandledrejection", t: now(), reason: String(e.reason) }));

async function ensureBeamr() { if (!beamr) beamr = await loadBundle(); return beamr; }

self.onmessage = async (e) => {
  const { cmd, origin } = e.data || {};
  try {
    const b = await ensureBeamr();

    if (cmd === "panic-1b") {
      // 1b: uncaught SYNC panic inside the Worker.
      const beam = await fetchBeam(`${origin}/artifacts/panic_probe.beam`);
      const vm = b.create_vm();
      vm.install_probe_panic_bif();
      vm.load_module(beam);
      b.register_panic_callback((payload) =>
        self.postMessage({ type: "cb", t: now(), payload: String(payload) }));
      const pid = vm.spawn("panic_probe", "boom", "[]");
      self.postMessage({ type: "armed", t: now() });
      // Drive uncaught in a macrotask so the trap routes to self.onerror.
      setTimeout(() => { vm.run_step(); }, 0);
      return;
    }

    if (cmd === "strand") {
      // WPORT-7 §2 strand measurement INSIDE the Worker. The VM and the timer
      // spies live in the Worker's global scope (spy installed at module top,
      // before any create_vm). Identical 20-sample logic as browser-main/Node.
      const strandBeam = await fetchBeam(`${origin}/artifacts/strand_probe.beam`);
      const result = await runStrandSamples({ beamr: b, spy, strandBeam, environment: "worker" });
      self.postMessage({ type: "strand-result", t: now(), data: result });
      return;
    }

    if (cmd === "throttle-arm") {
      // WPORT-3 run (b): arm BOTH deadline classes inside the Worker, then the
      // owner page backgrounds. Report armed spy state; report delivery later.
      const beam = await fetchBeam(`${origin}/artifacts/throttle_probe.beam`);
      const vm = b.create_vm();
      vm.load_module(beam);
      const armBefore = spy.arms.length;
      const recvPid = vm.spawn("throttle_probe", "wait30", "[]");      // T+30s receive-after
      const nativePid = vm.spawn("throttle_probe", "deliver45", "[]"); // T+45s native Deliver (self-armed)
      // Pump both to their parked/armed state.
      const settle = async () => {
        // await_exit resolves 'idle' while a deadline is pending; poll both
        // once to force the arm, then keep the VM alive for the throttle window.
        vm.run_step();
      };
      await settle();
      const armed = spy.arms.slice(armBefore);
      self.postMessage({ type: "throttle-armed", t: now(), armedDelays: armed.map((a) => a.delay),
        recvPid: Number(recvPid), nativePid: Number(nativePid) });
      // Race both exits; report as they resolve (late-but-delivered).
      const started = now();
      vm.await_exit(recvPid).then((r) => self.postMessage({ type: "throttle-fire", which: "receive-after",
        t: now(), elapsedMs: now() - started, completion: JSON.parse(r) }));
      vm.await_exit(nativePid).then((r) => self.postMessage({ type: "throttle-fire", which: "native-deliver",
        t: now(), elapsedMs: now() - started, completion: JSON.parse(r) }));
      return;
    }
  } catch (err) {
    self.postMessage({ type: "worker-error", t: now(), message: String((err && err.stack) || err) });
  }
};

self.postMessage({ type: "ready", t: now() });

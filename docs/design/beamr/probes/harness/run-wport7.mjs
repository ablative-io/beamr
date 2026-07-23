// WPORT-7 PROBE-FAILURE runner — headless. Drives web/wport7.html (main-thread
// legs 1a/1c/1d/1e + §2 strand) and web/wport7-worker.html (leg 1b). Records
// observations/wport7-<leg>.json; exits nonzero on suite-fatal. Smoke runs are
// marked {"smoke": true}.
import { recordObservation } from "./driver.mjs";
import { bootProbe, withTimeout } from "./runner-common.mjs";

const SMOKE = !process.argv.includes("--official");

async function runMain() {
  const probe = await bootProbe("wport7.html");
  globalThis.__lastProbe = probe;
  const complete = await withTimeout(probe.suite, 120000, "wport7 main suite");
  // Attach the CDP console log to the 1e observation (the platform-truth
  // stream-ordering channel): Node splits stdout/stderr; a browser console
  // shows one interleaved timeline. Record the actual devtools console order.
  const consoleProbeLines = probe.state.console
    .filter((c) => (c.args || []).some((a) => String(a).includes("probe-")))
    .map((c) => ({ t: c.t, type: c.type, text: (c.args || []).map(String).join(" ") }));

  const legs = {
    "strand": probe.state.obs.get("strand"),
    "console-ordering": {
      ...(probe.state.obs.get("console-ordering") || {}),
      cdpConsoleOrder: consoleProbeLines,
      cdpConsoleNote: "type 'log' == console.log (out), type 'error' == console.error (err). This is how the real browser devtools console timeline ordered the interleaved streams.",
    },
    "1a-caught": probe.state.obs.get("1a-caught"),
    "1a-uncaught": probe.state.obs.get("1a-uncaught"),
    "1c-queued-turn": probe.state.obs.get("1c-queued-turn"),
    "1d-latched-brick": probe.state.obs.get("1d-latched-brick"),
    "spy-summary": probe.state.obs.get("spy-summary"),
  };
  for (const [leg, data] of Object.entries(legs)) {
    recordObservation(`wport7-${leg}`, { smoke: SMOKE, probe: "wport7", leg, capturedAt: new Date().toISOString(), data });
  }
  recordObservation("wport7-suite", { smoke: SMOKE, probe: "wport7", complete, capturedAt: new Date().toISOString() });
  console.log("wport7 main: SUITE COMPLETE ->", Object.keys(legs).join(", "));
  console.log("  strand p50Ms:", legs.strand?.latency?.p50, "| allExitedGotProbe:", legs.strand?.allExitedGotProbe,
    "| 1a caughtRuntimeError:", legs["1a-caught"]?.caughtRuntimeError,
    "| 1a uncaught onerror:", legs["1a-uncaught"]?.windowOnerrorFired,
    "| 1d freshVmWorks:", legs["1d-latched-brick"]?.freshVmWorks);
  await probe.close();
}

async function runWorker() {
  const probe = await bootProbe("wport7-worker.html");
  const complete = await withTimeout(probe.suite, 60000, "wport7 worker suite");
  recordObservation("wport7-1b-worker", { smoke: SMOKE, probe: "wport7-worker", leg: "1b-worker",
    capturedAt: new Date().toISOString(), data: probe.state.obs.get("1b-worker") });
  recordObservation("wport7-worker-suite", { smoke: SMOKE, probe: "wport7-worker", complete, capturedAt: new Date().toISOString() });
  const d = probe.state.obs.get("1b-worker") || {};
  console.log("wport7 worker(1b): SUITE COMPLETE | callbackFired:", d.panicCallbackFired,
    "| workerSelfOnerror:", d.workerSelfOnerrorFired, "| pageWorkerOnerror:", d.ownerPageWorkerOnerrorFired,
    "| callbackBeforeTrap:", d.callbackBeforeTrap);
  await probe.close();
}

async function runStrandWorker() {
  const probe = await bootProbe("wport7-strand-worker.html");
  globalThis.__lastProbe = probe;
  const complete = await withTimeout(probe.suite, 90000, "wport7 strand-worker suite");
  const data = probe.state.obs.get("strand-worker");
  recordObservation("wport7-strand-worker", { smoke: SMOKE, probe: "wport7", leg: "strand-worker",
    capturedAt: new Date().toISOString(), data });
  console.log("wport7 strand-worker: SUITE COMPLETE | p50Ms:", data?.latency?.p50, "| maxMs:", data?.latency?.max,
    "| maxArmsDuringWait:", data?.maxArmsDuringWait, "| allExitedGotProbe:", data?.allExitedGotProbe);
  await probe.close();
}

async function main() {
  try {
    if (process.argv.includes("--strand-worker")) {
      await runStrandWorker();
      process.exit(0);
    }
    await runMain();
    await runWorker();
    process.exit(0);
  } catch (err) {
    recordObservation("wport7-suite-fatal", { smoke: SMOKE, probe: "wport7", error: String(err.stack || err), capturedAt: new Date().toISOString() });
    console.error("wport7: SUITE FATAL:", err.message);
    if (globalThis.__lastProbe) {
      console.error("  last trace:", JSON.stringify(globalThis.__lastProbe.state.obs.get("trace")));
      console.error("  page console tail:");
      for (const c of globalThis.__lastProbe.state.console.slice(-12)) console.error("   ", c.type, JSON.stringify(c.args));
    }
    process.exit(1);
  }
}
main();

// WPORT-3 PROBE-THROTTLE runner — HEADED (operator's display). Opens
// web/wport3.html, waits for the page's "armed" obs, backgrounds the tab
// (real visibilitychange + window minimize), sleeps past both deadlines AND
// the ~5-min intensive-throttle threshold, foregrounds, then waits for the
// final obs. Records observations/wport3-<leg>.json.
//
//   node run-wport3.mjs            # run (a) backgrounded tab, main-thread VM
//   node run-wport3.mjs --worker   # run (b) same workload inside a dedicated Worker
//
// DO NOT run headless: real platform throttling requires a real window that
// appears on the operator's display. This is the operator's run, not the
// harness author's.
import { recordObservation, backgroundTarget, foregroundTarget, sleep } from "./driver.mjs";
import { bootProbe, withTimeout } from "./runner-common.mjs";

const SMOKE = !process.argv.includes("--official");
const WORKER = process.argv.includes("--worker");
const BACKGROUND_MS = Number(process.env.WPORT3_BACKGROUND_MS || 6.5 * 60 * 1000); // 6.5 min
const ARMED_TIMEOUT_MS = 30000;
const POST_FOREGROUND_GRACE_MS = Number(process.env.WPORT3_GRACE_MS || 120000);

async function main() {
  const page = WORKER ? "wport3.html?mode=worker" : "wport3.html";
  const mode = WORKER ? "worker" : "main";
  const fires = [];
  let armedData = null, resolveArmed;
  const armed = new Promise((res) => { resolveArmed = res; });

  const probe = await bootProbe(page, {
    headed: true,
    onObs: (name, data) => {
      if (name === "armed") { armedData = data; resolveArmed(data); }
      if (name === "fire") fires.push(data);
    },
  });

  try {
    console.log(`wport3 (${mode}): waiting for armed ...`);
    await withTimeout(armed, ARMED_TIMEOUT_MS, "wport3 armed");
    recordObservation(`wport3-${mode}-armed`, { smoke: SMOKE, probe: "wport3", mode, leg: "armed", capturedAt: new Date().toISOString(), data: armedData });
    console.log(`wport3 (${mode}): ARMED unifiedDelayMs=${armedData?.unifiedArmedDelayMs}; backgrounding for ${Math.round(BACKGROUND_MS / 1000)}s`);

    const bg = await backgroundTarget(probe.cdp, probe.page.targetId);
    const bgStart = Date.now();
    await sleep(BACKGROUND_MS);
    console.log(`wport3 (${mode}): foregrounding after ${Math.round((Date.now() - bgStart) / 1000)}s`);
    await foregroundTarget(probe.cdp, probe.page.targetId, bg.windowId);

    const complete = await withTimeout(probe.suite, POST_FOREGROUND_GRACE_MS, "wport3 settle");
    const settled = probe.state.obs.get("settled");

    recordObservation(`wport3-${mode}-fires`, { smoke: SMOKE, probe: "wport3", mode, leg: "fire",
      capturedAt: new Date().toISOString(), backgroundMs: BACKGROUND_MS, fires });
    recordObservation(`wport3-${mode}-settled`, { smoke: SMOKE, probe: "wport3", mode, leg: "settled",
      capturedAt: new Date().toISOString(), backgroundMs: BACKGROUND_MS, data: settled, complete });

    console.log(`wport3 (${mode}): SETTLED lateButComplete=${settled?.lateButComplete} fires=${fires.length}`);
    for (const f of fires) console.log(`   fire ${f.which}: sinceArmMs=${Math.round(f.sinceArmMs ?? f.elapsedSinceStartMs ?? 0)} result=${JSON.stringify(f.completion?.result)}`);
    await probe.close();
    process.exit(0);
  } catch (err) {
    recordObservation(`wport3-${mode}-fatal`, { smoke: SMOKE, probe: "wport3", mode, error: String(err.stack || err),
      capturedAt: new Date().toISOString(), armed: armedData, firesSoFar: fires });
    console.error(`wport3 (${mode}): FATAL:`, err.message);
    await probe.close();
    process.exit(1);
  }
}
main();

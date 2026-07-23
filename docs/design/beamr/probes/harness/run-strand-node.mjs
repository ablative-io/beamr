// WPORT-7 §2 strand — NODE BASELINE. Runs the SAME 20-sample strand
// measurement as browser-main and the Worker, but on the --target web bundle
// under plain Node (no Chrome). Node 26 provides global fetch/WebAssembly; the
// bundle is initialized with an explicit wasm path (we read beamr_wasm_bg.wasm
// ourselves and hand the bytes to init). Zero deps, no re-bindgen.
//
//   node run-strand-node.mjs            # writes observations/wport7-strand-node.json (smoke:true)
//   node run-strand-node.mjs --official # operator overwrite
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { recordObservation } from "./driver.mjs";
import { installTimerSpies, runStrandSamples } from "./web/probe-common.js";

const SMOKE = !process.argv.includes("--official");
const HERE = dirname(fileURLToPath(import.meta.url));
const PKG = join(HERE, "web", "pkg", "beamr_wasm.js");
const WASM = join(HERE, "web", "pkg", "beamr_wasm_bg.wasm");
const STRAND_BEAM = join(HERE, "workloads", "strand_probe.beam");

async function main() {
  let beamr;
  try {
    beamr = await import(PKG);
    await beamr.default({ module_or_path: readFileSync(WASM) });
  } catch (err) {
    // Honor the instruction: if the --target web glue genuinely cannot
    // initialize under Node, report the exact error and STOP — never
    // re-bindgen a second bundle or add deps.
    console.error("run-strand-node: the --target web bundle FAILED to initialize under Node.");
    console.error("  exact error:", String(err && err.stack || err));
    recordObservation("wport7-strand-node", { smoke: SMOKE, probe: "wport7", leg: "strand-node",
      capturedAt: new Date().toISOString(), initFailed: true, error: String(err && err.stack || err) });
    process.exit(1);
  }

  // Spies on globalThis BEFORE the first create_vm (the VM captures
  // globalThis.setTimeout/clearTimeout at construction).
  const spy = installTimerSpies(globalThis);
  const strandBeam = new Uint8Array(readFileSync(STRAND_BEAM));

  const result = await runStrandSamples({ beamr, spy, strandBeam, environment: "node" });
  recordObservation("wport7-strand-node", { smoke: SMOKE, probe: "wport7", leg: "strand-node",
    capturedAt: new Date().toISOString(), nodeVersion: process.version, data: result });
  console.log("wport7 strand-node: DONE | p50Ms:", result.latency.p50, "| maxMs:", result.latency.max,
    "| maxArmsDuringWait:", result.maxArmsDuringWait, "| allExitedGotProbe:", result.allExitedGotProbe);
  console.log("  latency:", JSON.stringify(result.latency));
  process.exit(0);
}
main();

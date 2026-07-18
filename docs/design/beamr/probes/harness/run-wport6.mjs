// WPORT-6 PROBE-FETCH runner — headless. Drives web/wport6.html, records each
// expected observation to observations/wport6-<leg>.json, exits nonzero on
// suite-fatal. Smoke runs are marked {"smoke": true}; the official run
// (operator) overwrites them.
import { recordObservation } from "./driver.mjs";
import { bootProbe, withTimeout } from "./runner-common.mjs";

const SMOKE = !process.argv.includes("--official");

async function main() {
  const probe = await bootProbe("wport6.html");
  try {
    const complete = await withTimeout(probe.suite, 60000, "wport6 suite");

    // Observation 2 is assembled here from the CDP Network log: the four
    // /artifacts/ requests during the successful load window (manifest + 3
    // beams), plus the bad-manifest window's requests.
    const artifactReqs = probe.state.network.filter((n) => n.url.includes("/artifacts/"));
    const win = probe.state.obs.get("network-window");
    const goodWindow = win ? artifactReqs.filter((n) => {
      // Network timestamps are CDP monotonic seconds; we cannot align them to
      // page performance.now() exactly, so record the full list and let the
      // reader confirm the four-request shape.
      return true;
    }) : artifactReqs;

    const legs = {
      "report": probe.state.obs.get("report"),
      "run": probe.state.obs.get("run"),
      "network": {
        allArtifactRequests: artifactReqs.map((n) => n.url),
        artifactRequestCount: artifactReqs.length,
        note: "Expect 4 successful-load requests (manifest.json + fetch_chain_c/b/a.beam, deps before dependants) plus the bad-manifest leg's requests (manifest_bad.json + c + b + the 404 miss). Each good artifact fetched exactly once.",
        window: win || null,
      },
      "bad-url-rejection": probe.state.obs.get("bad-url-rejection"),
      "module-provenance-gap": probe.state.obs.get("module-provenance-gap"),
      "spy-summary": probe.state.obs.get("spy-summary"),
    };

    for (const [leg, data] of Object.entries(legs)) {
      recordObservation(`wport6-${leg}`, { smoke: SMOKE, probe: "wport6", leg, capturedAt: new Date().toISOString(), data });
    }
    recordObservation("wport6-suite", { smoke: SMOKE, probe: "wport6", complete, capturedAt: new Date().toISOString() });
    console.log("wport6: SUITE COMPLETE ->", Object.keys(legs).join(", "));
    console.log("  report.orderIsCBA:", legs.report?.orderIsCBA, "| run.resultIs42:", legs.run?.resultIs42,
      "| bad-url starts artifact_fetch_failed:", legs["bad-url-rejection"]?.messageStartsArtifactFetchFailed,
      "| artifactRequests:", legs.network.artifactRequestCount);
    await probe.close();
    process.exit(0);
  } catch (err) {
    recordObservation("wport6-suite-fatal", { smoke: SMOKE, probe: "wport6", error: String(err.stack || err), capturedAt: new Date().toISOString() });
    console.error("wport6: SUITE FATAL:", err.message);
    await probe.close();
    process.exit(1);
  }
}
main();

// Platform witness: does setTimeout(fn, N) ever run fn while
// performance.now() has advanced by LESS than N ms since registration?
//
// This is exactly the clock pair the beamr-wasm HostDeadlineService lives on:
// deadlines are stamped and compared with web_time::Instant (performance.now()
// under wasm32-unknown-unknown), while the one-shot arm is a host setTimeout
// (libuv's ms-granularity cached loop clock in Node). If the host can fire
// early relative to performance.now(), an armed 25ms deadline can find an
// empty due set at `record.deadline <= now` and the completion seam re-arms.
//
// Mimics the test shape: 25ms delay, registration happening mid-turn after
// some synchronous work (so libuv's cached loop time is stale).

const DELAY_MS = 25;
const SAMPLES = 400;

let early = 0;
let worst = Infinity;
const earlyDeltas = [];

function busyWork(ms) {
  // Synchronous work between loop-iteration start and registration,
  // matching a drain turn running before the arm.
  const until = performance.now() + ms;
  while (performance.now() < until) {}
}

function sample(i) {
  if (i >= SAMPLES) {
    console.log(JSON.stringify({
      node: process.version,
      delay_ms: DELAY_MS,
      samples: SAMPLES,
      early_fires: early,
      // Field names fixed post-tear (Waffles' ruling at the 2026-07-23
      // landing): the as-run outputs of that date carry the old name
      // `worst_early_ms`, which actually held the worst ELAPSED time at an
      // early fire, not the worst delta. Future runs emit both, named for
      // what they hold; the committed 2026-07-23 outputs stay as-run.
      worst_early_elapsed_ms: worst === Infinity ? null : Number(worst.toFixed(3)),
      worst_early_delta_ms:
        worst === Infinity ? null : Number((DELAY_MS - worst).toFixed(3)),
      early_deltas_ms: earlyDeltas.slice(0, 20).map(d => Number(d.toFixed(3))),
    }, null, 2));
    return;
  }
  // Vary the mid-turn work a little so registration lands at varied
  // sub-ms offsets within the loop iteration.
  setTimeout(() => {
    busyWork((i % 10) * 0.13);
    const t0 = performance.now();
    setTimeout(() => {
      const dt = performance.now() - t0;
      if (dt < DELAY_MS) {
        early += 1;
        earlyDeltas.push(DELAY_MS - dt);
        if (dt < worst) worst = dt;
      }
      sample(i + 1);
    }, DELAY_MS);
  }, 0);
}

sample(0);

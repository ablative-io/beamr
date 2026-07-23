// Cascade witness: when setTimeout fires EARLY relative to performance.now(),
// the HostDeadlineService re-arms one one-shot for the remaining delta,
// computed exactly like millis_until_ceil: ceil(remaining_micros / 1000),
// saturating at 0. This witness measures how long that re-arm CHAIN can get —
// the "geometrically bounded" claim demonstrated rather than asserted.
//
// Chain protocol per sample:
//   stamp deadline = performance.now() + DELAY_MS (sub-ms precision, exactly
//   the service's stamp-at-sync), then arm ceil(remaining) and on each fire:
//   if performance.now() < deadline -> hop: re-arm ceil(remaining) again.
//   Chain length = number of EXTRA arms beyond the first (0 = fired on time).

const DELAY_MS = 25;
const SAMPLES = 400;

let sampleIdx = 0;
const chainLengths = [];
const chainDeltas = []; // for early samples: [delta_before_each_rearm...]

function busyWork(ms) {
  const until = performance.now() + ms;
  while (performance.now() < until) {}
}

function ceilMillisUntil(deadline) {
  const remainingMicros = Math.max(0, (deadline - performance.now()) * 1000);
  return Math.ceil(remainingMicros / 1000); // millis_until_ceil, in ms
}

function runChain(deadline, hops, deltas, done) {
  setTimeout(() => {
    const now = performance.now();
    if (now < deadline) {
      deltas.push(deadline - now);
      runChain(deadline, hops + 1, deltas, done); // re-arm the remainder
      return;
    }
    done(hops, deltas);
  }, ceilMillisUntil(deadline));
}

function sample() {
  if (sampleIdx >= SAMPLES) {
    const early = chainLengths.filter(c => c > 0);
    console.log(JSON.stringify({
      node: process.version,
      platform: process.platform,
      delay_ms: DELAY_MS,
      samples: SAMPLES,
      early_samples: early.length,
      max_chain_length: Math.max(0, ...chainLengths),
      chain_length_histogram: chainLengths.reduce((h, c) => {
        h[c] = (h[c] || 0) + 1; return h;
      }, {}),
      early_deltas_ms: chainDeltas.flat().map(d => Number(d.toFixed(3))),
    }, null, 2));
    return;
  }
  setTimeout(() => {
    busyWork((sampleIdx % 10) * 0.13); // stale the libuv cached clock a little
    const deadline = performance.now() + DELAY_MS;
    runChain(deadline, 0, [], (hops, deltas) => {
      chainLengths.push(hops);
      if (hops > 0) chainDeltas.push(deltas);
      sampleIdx += 1;
      sample();
    });
  }, 0);
}

sample();

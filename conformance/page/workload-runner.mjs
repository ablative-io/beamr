// WPORT-9 shared workload runner — drives the committed conformance
// workload through a REAL bundle VM in any host (Node process, browser
// page, dedicated Worker). Environment-agnostic: the caller supplies the
// timer originals so the browser legs never pollute their own F-0d shim
// record, and every wait is settlement-event-driven — a timer-driven
// probe loop would itself be the polling this gate exists to kill.

// Settlement tracker: capability promises announce their settlement one
// original-macrotask later (so the arbiter's completion turn runs first).
export function makeTracker(originals) {
  const waiters = [];
  const announce = () => {
    const batch = waiters.splice(0, waiters.length);
    for (const waiter of batch) waiter();
  };
  return {
    settled() {
      originals.setTimeout(announce, 0);
    },
    nextSettlement() {
      return new Promise((resolve) => {
        waiters.push(resolve);
      });
    },
  };
}

// Settled-exit loop (WPORT-2 settled-idle contract): await_exit resolves
// a JSON envelope; `state: "idle"` means the process is still alive with
// the VM settled (parked on a capability op or a plain receive). On
// idle, wait for the next capability settlement (or perform the entry's
// one wake action), then probe again. Never timer-driven. Returns the
// parsed terminal envelope ({pid, reason, result, state, summary}).
export async function settledExit(vm, pid, tracker, originals, options = {}) {
  const deadlineMs = options.deadlineMs ?? 90000;
  let wake = options.wake ?? null;
  const watchdog = new Promise((_, reject) => {
    originals.setTimeout(
      () => reject(new Error(`settledExit watchdog (${deadlineMs}ms) pid ${pid}`)),
      deadlineMs,
    );
  });
  for (;;) {
    const outcome = await Promise.race([vm.await_exit(pid), watchdog]);
    const envelope = typeof outcome === "string" ? JSON.parse(outcome) : outcome;
    if (envelope?.state !== "idle") return envelope;
    if (wake) {
      const action = wake;
      wake = null;
      action();
      await new Promise((resolve) => originals.setTimeout(resolve, 0));
      continue;
    }
    await Promise.race([tracker.nextSettlement(), watchdog]);
    await new Promise((resolve) => originals.setTimeout(resolve, 0));
  }
}

// Fetch capability object (WPORT-8 R1 contract): request(requestObject,
// slot) -> thenable resolving the D8 response map shape. `fetchImpl`
// performs the actual transport (real fetch in browser/Worker legs,
// scripted in-memory response in the Node leg).
export function makeFetchCapability(fetchImpl, tracker) {
  return {
    request(requestObject, slot) {
      const controller = new AbortController();
      if (slot && typeof slot === "object") {
        slot.abort = () => controller.abort();
      }
      const work = Promise.resolve()
        .then(() => fetchImpl(requestObject, controller.signal))
        .finally(() => tracker.settled());
      return work;
    },
  };
}

// In-memory KV capability (WPORT-8 R1 contract). The REAL IndexedDB
// backend is sitting-territory by the ledger; the adapter contract is
// capability-object-shaped, which this satisfies hermetically.
export function makeKvCapability(tracker) {
  const store = new Map();
  const settleWith = (value) =>
    Promise.resolve(value).finally(() => tracker.settled());
  return {
    get(key) {
      return settleWith(store.has(key) ? store.get(key) : null);
    },
    put(key, value) {
      store.set(key, value);
      return settleWith(true);
    },
    delete(key) {
      store.delete(key);
      return settleWith(true);
    },
    list_by_prefix(prefix) {
      const keys = [...store.keys()].filter((k) => k.startsWith(prefix)).sort();
      return settleWith(keys);
    },
  };
}

function fail(name, detail) {
  return { name, ok: false, detail };
}

function pass(name, detail) {
  return { name, ok: true, detail };
}

function checkFields(name, value, expectations) {
  for (const [key, expected] of Object.entries(expectations)) {
    const actual = value?.[key];
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      return fail(name, `field ${key}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    }
  }
  return pass(name, value);
}

// Drive every workload entry through one VM; returns per-entry results.
// `env` supplies: vm, tracker, originals, fetchUrl (the loopback probe
// URL the capability_fetch entry hits).
export async function runWorkload(env) {
  const { vm, tracker, originals, fetchUrl } = env;
  const results = [];
  const spawnEntry = (fn, args) =>
    vm.spawn("wport9_conformance", fn, JSON.stringify(args));
  const exitOf = async (pid, options) => {
    const envelope = await settledExit(vm, pid, tracker, originals, options);
    return envelope?.result;
  };

  // wake_send: spawn edge + mailbox-send wake, all inside the VM.
  {
    const pid = spawnEntry("wake_send", []);
    const value = await exitOf(pid);
    results.push(checkFields("wake_send", value, {
      entry: "wake_send",
      child_value: 42,
      spawned: true,
    }));
  }

  // wake_cast: parks at true idle; the driver's cast is the wake.
  {
    const pid = spawnEntry("wake_cast", []);
    const value = await exitOf(pid, {
      wake: () => vm.cast(pid, { cast: "wport9-cast-payload" }),
    });
    results.push(checkFields("wake_cast", value, {
      entry: "wake_cast",
      payload: "wport9-cast-payload",
    }));
  }

  // wake_receive_timeout: the after-clause fires with nothing sent.
  {
    const pid = spawnEntry("wake_receive_timeout", []);
    const value = await exitOf(pid);
    results.push(checkFields("wake_receive_timeout", value, {
      entry: "wake_receive_timeout",
      outcome: "timed_out",
    }));
  }

  // wake_timer_deadline: send_after arms the wheel; delivery wakes.
  {
    const pid = spawnEntry("wake_timer_deadline", []);
    const value = await exitOf(pid);
    results.push(checkFields("wake_timer_deadline", value, {
      entry: "wake_timer_deadline",
      tick: true,
      cancelled_had_remaining: true,
    }));
  }

  // capability_fetch: promise-completion wake through the async-NIF seam.
  {
    const pid = spawnEntry("capability_fetch", [fetchUrl]);
    const value = await exitOf(pid);
    const response = value?.response;
    // The D8 response body is a binary; the exit-envelope JSON renders it
    // as an indexed byte object — decode either shape before asserting.
    const body = response?.body;
    const bodyText = typeof body === "string"
      ? body
      : String.fromCharCode(...Object.values(body ?? {}));
    if (response?.status === 200 && bodyText.includes("wport9") && response?.headers?.["x-wport9-probe"] === "ok") {
      results.push(pass("capability_fetch", { status: response.status, body: bodyText }));
    } else {
      results.push(fail("capability_fetch", JSON.stringify(value)));
    }
  }

  // capability_kv: put/get/list/delete round trip, lexicographic listing.
  {
    const pid = spawnEntry("capability_kv", ["wport9:alpha", "value-alpha"]);
    const value = await exitOf(pid);
    results.push(checkFields("capability_kv", value, {
      entry: "capability_kv",
      stored: "value-alpha",
      keys: ["wport9:alpha"],
      deleted: true,
    }));
  }

  // bif_supported: maps/comparison/self exercised as values.
  {
    const pid = spawnEntry("bif_supported", []);
    const value = await exitOf(pid);
    results.push(checkFields("bif_supported", value, {
      entry: "bif_supported",
      sum: 6,
      ordered: true,
      self_is_pid: true,
      keys: ["a", "b", "c"],
    }));
  }

  // bif_unsupported: statistics/1 refusal, catch-shaped, as a value.
  {
    const pid = spawnEntry("bif_unsupported", []);
    const value = await exitOf(pid);
    results.push(checkFields("bif_unsupported", value, {
      entry: "bif_unsupported",
      refusal: "badarg_caught",
    }));
  }

  return results;
}

// Output leg: two ordered writes through the registered sink.
export async function runOutputLeg(env) {
  const { vm, tracker, originals } = env;
  const lines = [];
  vm.register_io_sink((stream, text) => {
    lines.push(typeof text === "string" ? text : new TextDecoder().decode(text));
  });
  const pid = vm.spawn("wport9_conformance", "output_entry", "[]");
  const envelope = await settledExit(vm, pid, tracker, originals);
  const value = envelope?.result;
  const joined = lines.join("");
  const ordered =
    joined.indexOf("wport9 output line one") !== -1 &&
    joined.indexOf("wport9 output line one") < joined.indexOf("wport9 output line two");
  if (value?.wrote === 2 && ordered) {
    return pass("output-sink", { lines: joined });
  }
  return fail("output-sink", `exit=${JSON.stringify(value)} sink=${JSON.stringify(joined)}`);
}

// Process-error leg: the typed error surfaces, not just a dead pid —
// the envelope classifies errored AND take_exit_error carries the undef
// shape naming the module.
export async function runProcessErrorLeg(env) {
  const { vm, tracker, originals } = env;
  const pid = vm.spawn("wport9_conformance", "process_error", "[]");
  let envelope = null;
  try {
    envelope = await settledExit(vm, pid, tracker, originals);
  } catch (error) {
    // await_exit may reject for an errored process; the typed surface
    // below is the assertion either way.
  }
  const typed = vm.take_exit_error(pid);
  const text = typeof typed === "string" ? typed : JSON.stringify(typed);
  const errored = envelope === null || envelope?.state === "errored";
  if (errored && text && text.includes("undef") && text.includes("wport9_missing_module")) {
    return pass("process-error", { typed: text });
  }
  return fail("process-error", `state=${envelope?.state} typed surface: ${text}`);
}

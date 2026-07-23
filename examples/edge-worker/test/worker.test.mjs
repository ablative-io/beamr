import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { Miniflare } from "miniflare";

async function workerScript() {
  const source = await readFile(new URL("../src/worker.js", import.meta.url), "utf8");
  const stubBundle = `
    const vm = {
      nextPid: 1,
      results: new Map(),
      capabilities: {},
      register_fetch_capability(capability) { this.capabilities.fetch = capability; },
      register_kv_capability(capability) { this.capabilities.kv = capability; },
      spawn(module, fun, argsJson) {
        const pid = this.nextPid++;
        const [request] = JSON.parse(argsJson);
        const path = new URL(request.url).pathname;
        if (path === "/kv-roundtrip" || path === "/fetch-abort" || path === "/refusal") {
          this.results.set(pid, { capabilityProbe: path });
          return pid;
        }
        this.results.set(pid, {
          status: 201,
          headers: { "x-beamr": "edge", "x-beamr-pid": String(pid) },
          body: JSON.stringify({ module, fun, method: request.method, url: request.url, body: request.body })
        });
        return pid;
      },
      // Drive the REAL capability objects the worker registered — the wiring
      // under test is the worker's, not the stub's.
      async runCapabilityProbe(path) {
        if (path === "/refusal") {
          return ["error", ["capability_missing", "kv"]];
        }
        if (path === "/kv-roundtrip") {
          const kv = this.capabilities.kv;
          await kv.put("wport8:a1", "alpha");
          await kv.put("wport8:a2", "beta");
          const got = await kv.get("wport8:a1");
          const listed = await kv.list_by_prefix("wport8:");
          return { status: 200, headers: {}, body: JSON.stringify({ got, listed }) };
        }
        const slot = {};
        const pending = this.capabilities.fetch.request(
          { url: "https://blackhole.invalid/never", method: "GET", headers: {}, body: "" },
          slot
        );
        slot.abort();
        return pending.then(
          () => ({ status: 500, headers: {}, body: "unexpectedly resolved" }),
          (error) => ({
            status: 200,
            headers: {},
            body: JSON.stringify({ aborted: Boolean(error) && error.name === "AbortError" })
          })
        );
      },
      async await_exit(pid) {
        let value = this.results.get(pid) ?? null;
        this.results.delete(pid);
        if (value && value.capabilityProbe) {
          value = await this.runCapabilityProbe(value.capabilityProbe);
        }
        return JSON.stringify({
          state: value == null ? "idle" : "exited",
          pid,
          result: value,
          summary: {
            state: "idle",
            next_native_deadline_ms: null,
            runnable_remaining: 0,
            executed: value == null ? 0 : 1,
            yielded: [],
            waiting: [],
            exited: value == null ? [] : [pid],
            errored: [],
            results: value == null ? [] : [{ pid, value }]
          }
        });
      }
    };
    async function createPreloadedVm() {
      return { vm, loads: [] };
    }
    function parseJsonResult(value) {
      return typeof value === "string" ? JSON.parse(value) : value;
    }
    async function awaitExit(vm, pid) {
      return parseJsonResult(await vm.await_exit(pid));
    }
  `;
  return source.replace(
    'import { awaitExit, createPreloadedVm } from "../../../crates/beamr-wasm/pkg/beamr.bundle.mjs";',
    stubBundle
  );
}

test("Cloudflare Worker spawns one BEAM process per HTTP request shape", async () => {
  const miniflare = new Miniflare({
    modules: true,
    script: await workerScript(),
    bindings: {
      BEAMR_EDGE_MODULE: "edge_handler",
      BEAMR_EDGE_FUNCTION: "handle",
    },
  });
  try {
    const response = await miniflare.dispatchFetch("https://example.test/path", {
      method: "POST",
      body: "hello",
      headers: { "content-type": "text/plain" },
    });
    assert.equal(response.status, 201);
    assert.equal(response.headers.get("x-beamr"), "edge");
    const body = JSON.parse(await response.text());
    assert.deepEqual(body, {
      module: "edge_handler",
      fun: "handle",
      method: "POST",
      url: "https://example.test/path",
      body: "hello",
    });
  } finally {
    await miniflare.dispose();
  }
});

test("WebSocket upgrade stays out of scope", async () => {
  const miniflare = new Miniflare({ modules: true, script: await workerScript() });
  try {
    const response = await miniflare.dispatchFetch("https://example.test/socket", {
      headers: { upgrade: "websocket" },
    });
    assert.equal(response.status, 426);
  } finally {
    await miniflare.dispose();
  }
});

test("process exit results are consumed between requests", async () => {
  const miniflare = new Miniflare({ modules: true, script: await workerScript() });
  try {
    const first = await miniflare.dispatchFetch("https://example.test/first");
    const second = await miniflare.dispatchFetch("https://example.test/second");
    assert.equal(first.headers.get("x-beamr-pid"), "1");
    assert.equal(second.headers.get("x-beamr-pid"), "2");
    assert.equal(JSON.parse(await second.text()).url, "https://example.test/second");
  } finally {
    await miniflare.dispose();
  }
});

test("worker-registered KV capability round-trips against a real Miniflare namespace", async () => {
  const miniflare = new Miniflare({
    modules: true,
    script: await workerScript(),
    kvNamespaces: ["KV"],
  });
  try {
    const response = await miniflare.dispatchFetch("https://example.test/kv-roundtrip");
    assert.equal(response.status, 200);
    const body = JSON.parse(await response.text());
    assert.deepEqual(body, { got: "alpha", listed: ["wport8:a1", "wport8:a2"] });
  } finally {
    await miniflare.dispose();
  }
});

test("worker-registered fetch capability wires the abort hook to a real AbortController", async () => {
  const miniflare = new Miniflare({ modules: true, script: await workerScript() });
  try {
    const response = await miniflare.dispatchFetch("https://example.test/fetch-abort");
    assert.equal(response.status, 200);
    assert.deepEqual(JSON.parse(await response.text()), { aborted: true });
  } finally {
    await miniflare.dispose();
  }
});

test("a typed capability refusal surfaces as an observable HTTP error, never a hang", async () => {
  const miniflare = new Miniflare({ modules: true, script: await workerScript() });
  try {
    const response = await miniflare.dispatchFetch("https://example.test/refusal");
    assert.equal(response.status, 502);
    assert.deepEqual(JSON.parse(await response.text()), {
      error: { kind: "capability_missing", detail: "kv" },
    });
  } finally {
    await miniflare.dispose();
  }
});

test("worker.js calls no VM surface the real bundle does not export (stub-fidelity pin)", async () => {
  // WPORT-8 A4 rider 3: the test double must not drift from the real
  // surface. The bindgen source of truth is crates/beamr-wasm/src/lib.rs —
  // every JS-visible VM method is a `pub fn` there, so a name+arity check
  // against that superset can never false-fail on a real export while
  // catching any worker call the real bundle would not serve.
  const workerSource = await readFile(new URL("../src/worker.js", import.meta.url), "utf8");
  const bindgenSource = await readFile(
    new URL("../../../crates/beamr-wasm/src/lib.rs", import.meta.url),
    "utf8"
  );
  const exported = new Map();
  for (const match of bindgenSource.matchAll(/pub fn (\w+)\(([^)]*)\)/g)) {
    const params = match[2]
      .split(",")
      .map((param) => param.trim())
      .filter((param) => param.length > 0 && !/^&?\s*mut\s+self$|^&?\s*self$/.test(param));
    exported.set(match[1], params.length);
  }
  const callArity = (source, openIndex) => {
    let depth = 0;
    let args = 0;
    let sawToken = false;
    for (let i = openIndex; i < source.length; i += 1) {
      const ch = source[i];
      if (ch === "(") {
        depth += 1;
        if (depth === 1) continue;
      } else if (ch === ")") {
        depth -= 1;
        if (depth === 0) return sawToken ? args + 1 : 0;
      } else if (ch === "," && depth === 1) {
        args += 1;
        continue;
      }
      if (depth >= 1 && !/\s/.test(ch)) sawToken = true;
    }
    return null;
  };
  const calls = [...workerSource.matchAll(/vm\.(\w+)\(/g)];
  assert.ok(calls.length > 0, "worker.js drives the VM surface");
  for (const call of calls) {
    const name = call[1];
    assert.ok(
      exported.has(name),
      `worker.js calls vm.${name}() but the bindgen surface exports no such method`
    );
    const arity = callArity(workerSource, call.index + call[0].length - 1);
    assert.equal(
      arity,
      exported.get(name),
      `worker.js calls vm.${name}() with ${arity} args; the bindgen surface takes ${exported.get(name)}`
    );
  }
});

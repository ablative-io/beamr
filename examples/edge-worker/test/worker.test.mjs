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
      spawn(module, fun, argsJson) {
        const pid = this.nextPid++;
        const [request] = JSON.parse(argsJson);
        this.results.set(pid, {
          status: 201,
          headers: { "x-beamr": "edge", "x-beamr-pid": String(pid) },
          body: JSON.stringify({ module, fun, method: request.method, url: request.url, body: request.body })
        });
        return pid;
      },
      async await_exit(pid) {
        const value = this.results.get(pid) ?? null;
        this.results.delete(pid);
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

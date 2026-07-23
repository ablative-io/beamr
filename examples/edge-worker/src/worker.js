import { awaitExit, createPreloadedVm } from "../../../crates/beamr-wasm/pkg/beamr.bundle.mjs";

const DEFAULT_MODULE = "edge_handler";
const DEFAULT_FUNCTION = "handle";

let preloadedVmPromise;

function configuredModule(env = {}) {
  return env.BEAMR_EDGE_MODULE || DEFAULT_MODULE;
}

function configuredFunction(env = {}) {
  return env.BEAMR_EDGE_FUNCTION || DEFAULT_FUNCTION;
}

function getPreloadedVm() {
  if (!preloadedVmPromise) {
    preloadedVmPromise = createPreloadedVm();
  }
  return preloadedVmPromise;
}

function headersToObject(headers) {
  const object = Object.create(null);
  for (const [name, value] of headers) {
    object[name] = value;
  }
  return object;
}

async function requestToBeamValue(request) {
  const body = request.method === "GET" || request.method === "HEAD" ? "" : await request.text();
  return {
    method: request.method,
    url: request.url,
    headers: headersToObject(request.headers),
    body,
  };
}

function jsonSummary(value) {
  return typeof value === "string" ? JSON.parse(value) : value;
}

function responseFromBeamValue(value) {
  // A BEAM capability failure value ({error, {Slug, Detail}} — a two-element
  // JSON array under the "error" tag) surfaces as an observable typed HTTP
  // error, never a hang and never a silent 200.
  if (Array.isArray(value) && value[0] === "error") {
    const [kind, detail] = Array.isArray(value[1])
      ? value[1]
      : [String(value[1]), ""];
    return Response.json({ error: { kind, detail } }, { status: 502 });
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const status = Number(value.status ?? 200);
    if (!Number.isInteger(status) || status < 200 || status > 599) {
      throw new Error(`BEAM handler returned invalid HTTP status ${value.status}`);
    }
    const headers = new Headers();
    if (value.headers != null) {
      if (typeof value.headers !== "object" || Array.isArray(value.headers)) {
        throw new Error("BEAM handler returned invalid HTTP headers");
      }
      for (const [name, headerValue] of Object.entries(value.headers)) {
        if (headerValue != null) {
          headers.set(name, String(headerValue));
        }
      }
    }
    const body = value.body == null ? "" : value.body;
    return new Response(typeof body === "string" ? body : JSON.stringify(body), { status, headers });
  }
  if (typeof value === "string") {
    return new Response(value, { status: 200 });
  }
  return Response.json(value);
}

// WPORT-8 capability wiring: the platform's fetch and KV binding arrive as
// HOST-INJECTED capability objects — the BEAM side reaches no ambient global.
function fetchCapability() {
  return {
    request(request, slot) {
      const controller = new AbortController();
      // The bridge fires this hook when the calling BEAM process dies with
      // the request still in flight (process-death auto-abort).
      slot.abort = () => controller.abort();
      const init = {
        method: request.method || "GET",
        headers: request.headers || {},
        signal: controller.signal,
      };
      if (request.body != null && request.body !== "") {
        init.body = request.body;
      }
      return fetch(request.url, init).then(async (response) => ({
        status: response.status,
        headers: headersToObject(response.headers),
        body: await response.text(),
      }));
    },
  };
}

function kvCapability(namespace) {
  return {
    get: (key) => namespace.get(key),
    put: (key, value) => namespace.put(key, value),
    delete: (key) => namespace.delete(key),
    list_by_prefix: async (prefix) => {
      const listed = await namespace.list({ prefix });
      return listed.keys.map((entry) => entry.name);
    },
  };
}

function registerCapabilities(vm, env) {
  // Idempotent last-wins (WPORT-8 R1): per-request re-registration binds the
  // capability objects to the CURRENT env without touching the MFA registry.
  // A worker env without a KV binding leaves the whole wasm_kv module
  // refusing typed ({error, {capability_missing, kv}}) — never undef,
  // never a hang.
  vm.register_fetch_capability(fetchCapability());
  if (env.KV) {
    vm.register_kv_capability(kvCapability(env.KV));
  }
}

async function runBeamRequest(request, env) {
  const { vm } = await getPreloadedVm();
  registerCapabilities(vm, env);
  const requestValue = await requestToBeamValue(request);
  const pid = vm.spawn(configuredModule(env), configuredFunction(env), JSON.stringify([requestValue]));
  const completion = await awaitExit(vm, pid);
  if (completion.state !== "exited") {
    return Response.json(
      { error: "beam process did not produce a response", summary: jsonSummary(completion.summary) },
      { status: 503 }
    );
  }
  return responseFromBeamValue(completion.result);
}

export default {
  async fetch(request, env = {}) {
    if (request.headers.get("upgrade")) {
      return new Response("WebSocket upgrades are not supported by this stateless Beamr edge worker", {
        status: 426,
      });
    }
    try {
      return await runBeamRequest(request, env);
    } catch (error) {
      return Response.json(
        { error: error instanceof Error ? error.message : String(error) },
        { status: 500 }
      );
    }
  },
};

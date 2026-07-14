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

async function runBeamRequest(request, env) {
  const { vm } = await getPreloadedVm();
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

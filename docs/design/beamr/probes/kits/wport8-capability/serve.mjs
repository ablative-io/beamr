#!/usr/bin/env node
// WPORT-8 sitting kit — command 1 of 3: serve.
//
// Zero-dependency (Node stdlib only). On first run it builds the REAL
// bundle (wasm-pack web target with the sitting handler embedded, then the
// single-file packager), then serves everything from ONE local origin:
//
//   /                        the sitting page
//   /bundle/*                the real generated bundle (pkg + bootstrap.js)
//   /probe/ok                200 + JSON body (leg 1 fetch target)
//   /probe/slow?ms=N         responds after N ms (legs 2b and 3)
//   /sitting-env.json        auto-captured environment block
//   POST /evidence/<name>    writes <name>.json into evidence-out/
//
// Prerequisites: rust + wasm32-unknown-unknown target, wasm-pack, node.
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync, globSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { hostname } from "node:os";

const KIT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(KIT_DIR, "../../../../../..");
const HANDLER_DIR = join(KIT_DIR, "handler");
const PKG_DIR = join(REPO_ROOT, "crates/beamr-wasm/pkg");
const EVIDENCE_OUT = join(KIT_DIR, "evidence-out");
const PORT = 8787;

function run(command, args, options = {}) {
  console.log(`[kit] ${command} ${args.join(" ")}`);
  execFileSync(command, args, { stdio: "inherit", cwd: REPO_ROOT, ...options });
}

// The kit serves the wasm-pack pkg + the generated bootstrap.js directly
// (the WPORT-6 sitting shape). The single-file beamr.bundle.mjs is NOT
// used: its no-argument init path is broken at the packager (blob-scope
// ReferenceError — flagged for its own lane, 2026-07-24).
function buildBundleIfMissing() {
  if (existsSync(join(PKG_DIR, "bootstrap.js")) && existsSync(join(PKG_DIR, "beamr_wasm_bg.wasm"))) {
    console.log(`[kit] bundle present under ${PKG_DIR} (delete the dir to force a rebuild)`);
    return;
  }
  run("wasm-pack", ["build", "crates/beamr-wasm", "--target", "web", "--out-dir", "pkg"], {
    env: { ...process.env, BEAMR_WASM_BUNDLE_DIR: HANDLER_DIR },
  });
  const bootstraps = globSync(
    join(REPO_ROOT, "target/wasm32-unknown-unknown/release/build/beamr-wasm-*/out/beamr-wasm-bundle/bootstrap.js"),
  );
  if (bootstraps.length === 0) {
    throw new Error("bootstrap.js not found under target/ after wasm-pack build");
  }
  const bootstrap = bootstraps
    .map((path) => ({ path, mtime: statSyncMs(path) }))
    .sort((a, b) => b.mtime - a.mtime)[0].path;
  copyFileSync(bootstrap, join(PKG_DIR, "bootstrap.js"));
  console.log(`[kit] staged ${bootstrap} -> ${join(PKG_DIR, "bootstrap.js")}`);
}

function statSyncMs(path) {
  return Number(execFileSync("stat", ["-f", "%m", path]).toString().trim());
}

function gitOutput(args) {
  return execFileSync("git", args, { cwd: REPO_ROOT }).toString().trim();
}

function environmentBlock() {
  return {
    serving_shape:
      "wasm-pack pkg + generated bootstrap.js served directly (WPORT-6 sitting shape); single-file beamr.bundle.mjs bypassed — its no-arg init path is broken at the packager, flagged 2026-07-24 for its own lane",
    bundle_commit: gitOutput(["rev-parse", "HEAD"]),
    bundle_branch: gitOutput(["rev-parse", "--abbrev-ref", "HEAD"]),
    working_tree_dirty_tracked_files: gitOutput(["status", "--porcelain", "-uno"]) !== "",
    node_version: process.version,
    wasm_pack_version: execFileSync("wasm-pack", ["--version"]).toString().trim(),
    os: `${process.platform} ${execFileSync("uname", ["-r"]).toString().trim()}`,
    host: hostname(),
    serve_origin: `http://127.0.0.1:${PORT}`,
  };
}

const CONTENT_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
};

function serveFile(res, path) {
  const extension = path.slice(path.lastIndexOf("."));
  res.setHeader("content-type", CONTENT_TYPES[extension] ?? "application/octet-stream");
  res.end(readFileSync(path));
}

buildBundleIfMissing();
mkdirSync(EVIDENCE_OUT, { recursive: true });
const environment = environmentBlock();

const server = createServer((req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
  try {
    if (req.method === "POST" && url.pathname.startsWith("/evidence/")) {
      const name = url.pathname.slice("/evidence/".length).replace(/[^A-Za-z0-9._-]/g, "_");
      let body = "";
      req.on("data", (chunk) => { body += chunk; });
      req.on("end", () => {
        const file = join(EVIDENCE_OUT, `${name}.json`);
        writeFileSync(file, body);
        console.log(`[kit] evidence written: ${file}`);
        res.end("ok");
      });
      return;
    }
    if (url.pathname === "/") return serveFile(res, join(KIT_DIR, "page/index.html"));
    if (url.pathname === "/sitting.js") return serveFile(res, join(KIT_DIR, "page/sitting.js"));
    if (url.pathname.startsWith("/bundle/")) {
      const name = url.pathname.slice("/bundle/".length);
      if (!/^[A-Za-z0-9._-]+$/.test(name)) {
        res.statusCode = 400;
        return res.end("bad bundle path");
      }
      return serveFile(res, join(PKG_DIR, name));
    }
    if (url.pathname === "/sitting-env.json") {
      res.setHeader("content-type", CONTENT_TYPES[".json"]);
      return res.end(JSON.stringify(environment, null, 2));
    }
    if (url.pathname === "/probe/ok") {
      res.setHeader("content-type", CONTENT_TYPES[".json"]);
      res.setHeader("x-wport8-probe", "ok");
      return res.end(JSON.stringify({ probe: "ok" }));
    }
    if (url.pathname === "/probe/slow") {
      const ms = Number(url.searchParams.get("ms") ?? "1500");
      setTimeout(() => {
        res.setHeader("x-wport8-probe", "slow");
        res.end("slow-body");
      }, ms);
      return;
    }
    res.statusCode = 404;
    res.end("not found");
  } catch (error) {
    res.statusCode = 500;
    res.end(String(error));
  }
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`[kit] sitting page: http://127.0.0.1:${PORT}/`);
  console.log(`[kit] environment: ${JSON.stringify(environment)}`);
  console.log("[kit] open DevTools -> Network BEFORE pressing the run button");
});

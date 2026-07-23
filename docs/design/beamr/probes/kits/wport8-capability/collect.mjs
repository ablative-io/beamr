#!/usr/bin/env node
// WPORT-8 sitting kit — command 3 of 3: collect.
//
// Copies the evidence JSONs the sitting page wrote (evidence-out/) into the
// probes evidence tree under a dated directory, then prints the explicit
// staging command (explicit paths only — the kit never runs git for you).
import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const KIT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(KIT_DIR, "../../../../../..");
const EVIDENCE_OUT = join(KIT_DIR, "evidence-out");
const date = new Date().toISOString().slice(0, 10);
const DEST = join(REPO_ROOT, `docs/design/beamr/probes/evidence/${date}-capability-sitting`);

const files = readdirSync(EVIDENCE_OUT).filter((name) => name.endsWith(".json"));
if (files.length === 0) {
  console.error(`no evidence found in ${EVIDENCE_OUT} — run the sitting page first`);
  process.exit(1);
}
mkdirSync(DEST, { recursive: true });
for (const name of files.sort()) {
  copyFileSync(join(EVIDENCE_OUT, name), join(DEST, name));
  console.log(`staged: ${join(DEST, name)}`);
}
console.log("\nreview the JSONs, then stage explicitly:");
console.log(`  git add ${DEST.replace(`${REPO_ROOT}/`, "")}`);

---
name: norn
description: Delegate substantial repository work from Claude Code to a persistent headless Norn session using GPT-5.6 Sol. Use this skill whenever the user asks Norn to scout, research, review, or implement work; wants an independent GPT second opinion; wants to replace a Claude sub-agent; or needs structured, auditable delegation with a session ID and JSON Schema result.
argument-hint: "[scout|research|review|dev] [task] [preset] [medium|high|xhigh|max]"
compatibility: Requires `norn` and `jq` on PATH and a Norn Codex-subscription login.
---

# Norn Delegation

Use Norn as a persistent external worker for one of four modes:

| Mode | Default preset | Purpose |
|---|---|---|
| `scout` | `repository-map` | Locate code, map flows, and narrow an investigation |
| `research` | `evidence-synthesis` | Establish facts, compare options, and answer a question with evidence |
| `review` | `correctness` | Independently assess existing work without changing it |
| `dev` | `implementation` | Make a bounded implementation and verify it |

Every invocation persists a Norn session. Never add `--no-session`. Always
return the session ID and saved envelope path to the caller so the work can be
audited, resumed, or exported later.

## Defaults

- Model: `gpt-5.6-sol`
- Reasoning: `high`
- Service tier: `--fast`
- Output: mode-specific structured value inside Norn's JSON envelope
- Session: persistent, with a discoverable `claude-<mode>-...` name
- Working directory and built-in file-tool boundary: repository root

Use Sol only at `medium`, `high`, `xhigh`, or `max`. Default to `high`.
Choose `xhigh` for difficult security, concurrency, persistence, architecture,
or cross-module work. Reserve `max` for genuinely phase-decisive work. If the
job is too small to justify `medium`, use a smaller model rather than Sol at
low effort. Keep `--fast` enabled for now.

## Presets

Each mode loads `instructions/<mode>/base.md` plus one specialist preset:

### Scout

- `repository-map`: identify entry points, responsibilities, and major flows
- `change-impact`: trace the consequences of a proposed change
- `incident-triage`: localize a concrete failure from symptom to source

### Research

- `evidence-synthesis`: answer a question from verified evidence
- `implementation-options`: compare viable implementation paths and tradeoffs
- `external-landscape`: combine repository facts with current primary sources

### Review

- `mechanical`: find incomplete refactors, stale paths, policy violations, and drift
- `correctness`: find behavioral defects and unsupported invariants
- `safety`: assess trust boundaries, credentials, disclosure, and unsafe authority
- `concurrency`: inspect races, ordering, cancellation, and durability
- `architecture`: assess ownership, boundaries, coupling, and migration shape
- `test-evidence`: assess whether tests and evidence actually prove the claims

### Dev

- `implementation`: implement a bounded feature end to end
- `bug-fix`: reproduce, repair, and regression-test a defect
- `refactor`: improve structure without changing intended behavior
- `test-hardening`: add meaningful regression and adversarial coverage

## Run Norn

Fill in `MODE`, `TASK`, and `SCOPE`. Override `PRESET`, `EFFORT`, or
`SCHEMA_PATH` only when the task requires it.

```bash
set -euo pipefail
umask 077

MODE="<scout|research|review|dev>"
TASK="<the concrete outcome Norn must produce>"
SCOPE="<paths, commit range, requirements, symptoms, or constraints>"
EFFORT="${EFFORT:-high}"

WORKSPACE="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
SKILL_DIR="${NORN_SKILL_DIR:-}"
if [[ -z "$SKILL_DIR" ]]; then
  if [[ -d "$WORKSPACE/.claude/skills/norn" ]]; then
    SKILL_DIR="$WORKSPACE/.claude/skills/norn"
  else
    SKILL_DIR="$HOME/.claude/skills/norn"
  fi
fi
NORN_STATE_HOME="${NORN_HOME:-$HOME/.norn}"

case "$MODE" in
  scout)
    DEFAULT_PRESET="repository-map"
    ALLOWED_TOOLS="read,search,lsp,bash"
    ;;
  research)
    DEFAULT_PRESET="evidence-synthesis"
    ALLOWED_TOOLS="read,search,lsp,bash,web_search,web_fetch"
    ;;
  review)
    DEFAULT_PRESET="correctness"
    ALLOWED_TOOLS="read,search,lsp,bash"
    ;;
  dev)
    DEFAULT_PRESET="implementation"
    ALLOWED_TOOLS="read,search,lsp,bash,write,edit,apply_patch,action_log"
    ;;
  *)
    printf 'unknown Norn mode: %s\n' "$MODE" >&2
    exit 2
    ;;
esac

case "$EFFORT" in
  medium|high|xhigh|max) ;;
  *)
    printf 'unsupported Sol reasoning effort: %s\n' "$EFFORT" >&2
    exit 2
    ;;
esac

PRESET="${PRESET:-$DEFAULT_PRESET}"
SCHEMA_PATH="${SCHEMA_PATH:-$SKILL_DIR/schemas/$MODE.schema.json}"
BASE_INSTRUCTIONS="$SKILL_DIR/instructions/$MODE/base.md"
PRESET_INSTRUCTIONS="$SKILL_DIR/instructions/$MODE/$PRESET.md"

for required_file in "$SCHEMA_PATH" "$BASE_INSTRUCTIONS" "$PRESET_INSTRUCTIONS"; do
  if [[ ! -f "$required_file" ]]; then
    printf 'missing Norn skill resource: %s\n' "$required_file" >&2
    exit 2
  fi
done

if [[ "$NORN_STATE_HOME" != /* ]]; then
  printf 'NORN_HOME must be absolute for persistent delegation: %s\n' "$NORN_STATE_HOME" >&2
  exit 2
fi

ENVELOPE_DIR="$NORN_STATE_HOME/delegations"
mkdir -p "$ENVELOPE_DIR"
RESULT_FILE="$(mktemp "$ENVELOPE_DIR/claude-$MODE.XXXXXX")"
SESSION_NAME="claude-$MODE-$(date -u +%Y%m%dT%H%M%SZ)-$$"
APPENDED_SYSTEM_PROMPT="$(cat "$BASE_INSTRUCTIONS" "$PRESET_INSTRUCTIONS")"

PROMPT="$(cat <<PROMPT
Delegation mode: $MODE
Specialist preset: $PRESET
Repository: $WORKSPACE

Task:
$TASK

Scope and constraints:
$SCOPE

Work from the current repository state. Return only the value required by the
supplied output schema. Do not claim completion or verification without direct
evidence.
PROMPT
)"

norn --print \
  --model gpt-5.6-sol \
  --reasoning-effort "$EFFORT" \
  --fast \
  --working-dir "$WORKSPACE" \
  --workspace-root "$WORKSPACE" \
  --allowed-tools "$ALLOWED_TOOLS" \
  --append-system-prompt "$APPENDED_SYSTEM_PROMPT" \
  --session-name "$SESSION_NAME" \
  --quiet \
  --output-schema "$SCHEMA_PATH" \
  --output-format json \
  >"$RESULT_FILE" <<<"$PROMPT"

SESSION_ID="$(jq -er '.session_id' "$RESULT_FILE")"
printf 'Norn session: %s\n' "$SESSION_ID" >&2
printf 'Norn envelope: %s\n' "$RESULT_FILE" >&2

jq -e '
  if .stop.reason == "completed" then
    .output
  else
    error("Norn delegation did not complete; inspect the saved envelope")
  end
' "$RESULT_FILE"
```

The caller must retain and report both `SESSION_ID` and `RESULT_FILE`. Do not
delete the envelope automatically. Norn's own persisted session is the durable
record; the envelope is the convenient machine-readable handoff.

`--workspace-root` confines Norn's built-in file tools. It does not sandbox
`bash`. Scout, research, and review instructions prohibit repository mutation,
but use an OS sandbox or disposable read-only worktree when enforcement rather
than instruction is required.

## Custom Output Schemas

`--output-schema` constrains and locally validates the model's final value.
`--output-format json` wraps that value in `.output` alongside `.stop`,
`.usage`, `.session_id`, `.events`, and `.diagnostics`. Use both for delegation.

To customize a mode, copy its schema and set `SCHEMA_PATH` to the new file. A
portable schema should:

1. Be complete and self-contained. Do not use external `$ref` URLs or depend on
   another schema document.
2. Use a root object and set `additionalProperties: false` on every object.
3. Put every declared property in that object's `required` array. Prefer empty
   arrays or strings over optional object properties.
4. Use the portable structured-output subset: objects, arrays, strings,
   numbers, booleans, nulls, enums, descriptions, and simple bounds.
5. Avoid comments, trailing commas, `patternProperties`, conditionals, and
   advanced schema features unless the active backend is verified to support them.

The schema applies to `.output`, not to the surrounding CLI envelope.

## Resume For Follow-Up

Resume the persisted worker when the caller needs clarification or another pass.
Use the session ID printed by the original command and repeat all behavioral
flags explicitly; a new process does not inherit them.

```bash
set -euo pipefail
umask 077

SESSION_ID="<persisted Norn session id>"
FOLLOW_UP="<question, correction, or next bounded task>"
MODE="<same mode>"
PRESET="<same or new preset>"
EFFORT="${EFFORT:-high}"
WORKSPACE="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
SKILL_DIR="${NORN_SKILL_DIR:-}"
if [[ -z "$SKILL_DIR" ]]; then
  if [[ -d "$WORKSPACE/.claude/skills/norn" ]]; then
    SKILL_DIR="$WORKSPACE/.claude/skills/norn"
  else
    SKILL_DIR="$HOME/.claude/skills/norn"
  fi
fi
NORN_STATE_HOME="${NORN_HOME:-$HOME/.norn}"
SCHEMA_PATH="${SCHEMA_PATH:-$SKILL_DIR/schemas/$MODE.schema.json}"
BASE_INSTRUCTIONS="$SKILL_DIR/instructions/$MODE/base.md"
PRESET_INSTRUCTIONS="$SKILL_DIR/instructions/$MODE/$PRESET.md"
APPENDED_SYSTEM_PROMPT="$(cat "$BASE_INSTRUCTIONS" "$PRESET_INSTRUCTIONS")"
ENVELOPE_DIR="$NORN_STATE_HOME/delegations"
mkdir -p "$ENVELOPE_DIR"
RESULT_FILE="$(mktemp "$ENVELOPE_DIR/claude-$MODE-follow-up.XXXXXX")"

case "$MODE" in
  scout|review) ALLOWED_TOOLS="read,search,lsp,bash" ;;
  research) ALLOWED_TOOLS="read,search,lsp,bash,web_search,web_fetch" ;;
  dev) ALLOWED_TOOLS="read,search,lsp,bash,write,edit,apply_patch,action_log" ;;
  *) exit 2 ;;
esac

norn --print \
  --model gpt-5.6-sol \
  --reasoning-effort "$EFFORT" \
  --fast \
  --working-dir "$WORKSPACE" \
  --workspace-root "$WORKSPACE" \
  --allowed-tools "$ALLOWED_TOOLS" \
  --append-system-prompt "$APPENDED_SYSTEM_PROMPT" \
  --resume "$SESSION_ID" \
  --quiet \
  --output-schema "$SCHEMA_PATH" \
  --output-format json \
  >"$RESULT_FILE" <<<"$FOLLOW_UP"

printf 'Norn session: %s\n' "$SESSION_ID" >&2
printf 'Norn envelope: %s\n' "$RESULT_FILE" >&2
jq -e 'if .stop.reason == "completed" then .output else error("follow-up did not complete") end' "$RESULT_FILE"
```

For a free-form follow-up, omit `--output-schema` and still inspect `.output`,
`.stop.reason`, and `.diagnostics` in the envelope.

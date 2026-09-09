# Beamr strategic review

**Date:** 2026-09-03  
**Audience:** Beamr maintainers and adjacent-stack teams  
**Review basis:** the checked-out `pr20-numeric-eq` tree at `58a7e5d17915`, its tests and design documents, and the local `origin/main` reference at `43d87819ed37`. The checkout contains unrelated work in progress; this document does not assess or modify it.

## Executive view

Beamr is the stack's execution substrate: a Rust implementation of the useful BEAM machine, with its own scheduler, process model, mailboxes, garbage collector, bytecode loader, interpreter, JIT/AOT work, and host-capability boundary. That is a substantial and valuable core. Its principal strategic risk is not lack of features; it is an identity that can silently expand from "the BEAM subset needed by our programs" into "a replacement for Erlang/OTP."

That leaves a central product tension: Beamr is already more than a language experiment, but the evidence still describes selected language/runtime profiles more accurately than it describes undifferentiated BEAM/OTP compatibility.

Elixir is a compelling compatibility surface. It has a much larger commercial application base than Gleam, it exercises different bytecode and runtime semantics, and it would expose accidental Gleam-specific assumptions quickly. Corpus-level Elixir compatibility and arbitrary Phoenix-release compatibility are materially different claims.

## Place in the focused stack

| Project | Responsibility | Boundary visible in the architecture |
|---|---|---|
| Beamr | Execute BEAM bytecode and enforce runtime/capability rules | Execution rather than durable workflow, messaging, or storage semantics |
| Haematite | Durable local and replicated state | Integrate through an adapter, not storage types in VM APIs |
| Liminal | Addressed conversation and delivery semantics | Use Beamr processes where they add value without making the client protocol Beamr-specific |
| Aion | Durable workflow coordination and operator-facing product | Treat Beamr as one runtime backend behind a narrow interface |

The dependency direction places Beamr low in the stack; Aion and Liminal product concepts are not part of its VM model.

## What is already strong

- The implementation is a real VM rather than a wrapper around ERTS: process scheduling, heap ownership, messaging, code loading, interpreter execution, and native host functions are present.
- Targeting the bytecode actually emitted by selected source languages is a sensible way to bound a very large compatibility surface.
- The native capability boundary is strategically useful for controlled workflow execution.
- JIT/AOT work creates a route to differentiated performance and deployment rather than compatibility alone.
- The repository is unusually candid about its memory-safety history and remaining JIT-reachable risk.

## The main risks

### 1. Compatibility can be asserted more broadly than it is proved

The current README correctly describes a Gleam-targeted subset, but the implementation also contains compatibility shims that look like OTP support from the outside. For example, `supervisor:start_link/2` currently returns `{ok, self()}` rather than implementing supervisor semantics. It satisfies a narrowly proved call path, but it is evidence of a profile shim rather than general OTP compatibility.

### 2. The unsafe/JIT surface still sets the release ceiling

The published advisory says the remaining JIT-reachable sites do not yet have a clean bill of health, and that retaining threads also retains JIT. This qualifies every higher-level claim made by a threaded deployment regardless of how complete its language compatibility is.

### 3. Stack-wide type coupling amplifies Beamr churn

Haematite, Liminal, and Aion consume Beamr types or features directly. This turns a VM version move into a stack release train; the degree to which their public boundaries expose those types determines how far the coupling travels.

### 4. Record/replay and determinism need a single truthful contract

For a runtime beneath durable workflows, the relevant contract is which observations are reproducible, which host effects are recorded, and what invalidates a replay. Partial recorder plumbing can look like a stronger guarantee than it supplies.

## Elixir as a compatibility and type-system surface

Elixir is now a more interesting target than it was even a year ago. Elixir 1.20 completes the first milestone of its gradual set-theoretic type system: the compiler infers across language constructs, clauses, standard library calls, and dependencies to report verified violations. Typed structs, recursive and parametric types, and user-written function signatures remain future work. The upstream design is sound and deliberately avoids adding runtime checks merely to make gradual typing work.

Sources: [Elixir 1.20 release](https://elixir-lang.org/blog/2026/06/03/elixir-v1-20-0-released/), [type-inference roadmap](https://elixir-lang.org/blog/2026/01/09/type-inference-of-all-and-next-15/), and [gradual set-theoretic types](https://elixir.hexdocs.pm/main/gradual-set-theoretic-types.html).

This exposes two analytically separate surfaces.

### Elixir runtime compatibility

Elixir exercises a materially different portion of the platform from the present Gleam target. Its compiled applications bring pattern matching, guards, closures, exceptions and stacktraces, protocols, behaviours, structs, binaries, maps, comprehensions, links and monitors, `GenServer`, supervision, ETS, application configuration, code loading, and release boot into view. A pure Elixir module and a Phoenix release are therefore not two sizes of the same compatibility claim; they are different rings of OTP and tooling semantics.

The present `supervisor:start_link/2` shim illustrates the distinction. It is sufficient for a known Gleam call path because it returns the shape that path requires, but an Elixir application would attach lifecycle, restart, ancestry, and failure meaning to the same call. A corpus compiled by a pinned official Elixir/OTP toolchain would reveal these boundaries as emitted opcodes, imports, chunks, and behavioural expectations rather than as a generic count of supported instructions.

This is also why Elixir is a useful diagnostic target even apart from market size: it would expose where Beamr implements BEAM semantics, where it implements a Gleam-shaped subset, and where it currently supplies a compatibility-shaped return value.

### Type evidence in an alternate VM

The normal BEAM loader cannot assume debug information survives deployment: BEAM files are chunked, debug information is opaque to consumers, and release stripping removes it by default. See the official [`beam_lib` documentation](https://www.erlang.org/docs/28/apps/stdlib/beam_lib.html). Scraping unstable compiler internals from `Dbgi` would therefore not amount to a durable optimization contract.

An alternate VM creates an option that stock BEAM does not currently exploit: a versioned **typed execution manifest** emitted beside the BEAM module, or in a retained custom chunk. Such a representation has several necessary properties if it is to be trustworthy:

- bind it to the exact module bytecode hash and compiler/type-schema version;
- describe proven argument, return, local-value, clause, and shape facts at stable instruction or IR identities;
- distinguish closed-world proof from `dynamic()` ranges and unknown external calls;
- reject stale or unrecognized evidence and fall back to generic execution;
- never make type metadata necessary for semantic correctness;
- preserve guards, deoptimization, or generic fallback wherever hot code loading or dynamic calls can invalidate specialization.

Possible optimization consequences include unboxed numeric paths, redundant tag-test removal, tuple/map shape specialization, better call-site selection, and more precise GC root maps. None follows automatically from the presence of types; typed Elixir's primary upstream purpose is correctness and contracts, and José Valim has explicitly cautioned that ordinary Elixir-on-BEAM is not expected to gain meaningful performance merely from types.

The natural relationship is with the upstream Elixir type model rather than a competing annotation language. The Elixir compiler is the source of the proof; Beamr is a possible consumer of a stable exported artifact. Bytecode-hash binding, compiler/schema versioning, explicit treatment of `dynamic()`, and generic fallback are the difference between type evidence and an unsafe optimization hint.

## Cross-stack reading

Beamr is the lowest execution layer, but its types and release choices currently travel upward into Haematite, Liminal, and Aion. That produces a version train: VM safety work, feature changes, or representation changes can require coordinated updates in storage, messaging, and orchestration even where those products only need a small runtime interface.

The reverse pressure is also visible. Aion's deterministic workflow needs, Liminal's process model, and Haematite's cooperative execution can pull product-specific concerns down into the VM. The architectural question is therefore less "how complete is Beamr?" than "which semantics belong to Beamr, and which are profile or adapter semantics owned above it?"

The typed-Elixir idea makes that distinction especially clear. Type-guided instruction selection, root maps, and deoptimization are VM concerns. Whether a workflow may read time, create randomness, or launch unbounded work is an Aion/AWL language concern. Both use static knowledge, but they enforce different kinds of truth.

The current evidence presents Beamr as a genuinely substantial alternate runtime with three simultaneous identities: a Gleam-focused BEAM subset, an execution substrate for the Ablative stack, and an experimental route to native specialization. Elixir can connect those identities, but it also makes any ambiguity between them much easier to see.

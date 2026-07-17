# Visual content plan

Placement suggestions for screenshots, terminal recordings, and diagrams
that would strengthen beamr's documentation.

## README

| Location | Content | Format |
|---|---|---|
| After "Getting started" | Terminal recording: `cargo build --release -p beamr-cli` then running `beamr proof.beam proof:factorial/1 -- 20` showing the result | asciinema or GIF, ~15s |
| After "How it works" | Architecture diagram: `.gleam` source -> `gleam build` -> `.beam` bytecode -> beamr VM -> result, showing where beamr sits in the pipeline | SVG |
| After "Architecture" | Crate dependency diagram: `gleam-types` -> `beamr` -> `beamr-cli` / `beamr-wasm`, showing the workspace structure | SVG |
| After "What's implemented" | Diagram: the major VM subsystems (loader, interpreter, scheduler, GC, term representation, native BIFs) and how they connect | SVG |

## Architecture deep dive (docs/beamr-vm-design.md)

| Location | Content | Format |
|---|---|---|
| Top of document | High-level block diagram: bytecode loader -> module registry -> interpreter loop -> scheduler, with GC and mailbox interactions | SVG |
| Term representation section | Diagram of the 64-bit tagged pointer layout (3-bit low tag, payload bits) showing each term type | SVG or PNG |
| Scheduler section | Diagram: thread pool with work-stealing deques, reduction counting, dirty scheduler pool | SVG |
| GC section | Diagram: generational copying GC — young heap, old heap, minor/major collection triggers | SVG |

## ADR documents (docs/adr/)

| Location | Content | Format |
|---|---|---|
| ADR 004 (low-bit term tagging) | Bit-level layout diagram of the u64 tagged pointer scheme | SVG or ASCII art |
| ADR 008 (message passing copies terms) | Diagram: two process heaps with a term being deep-copied across | SVG |
| ADR 011 (lock-free mailbox) | Diagram: lock-free queue structure with enqueue/dequeue pointers | SVG |

## Distribution docs

| Location | Content | Format |
|---|---|---|
| DISTRIBUTION-HANDSHAKE-DESIGN.md | Sequence diagram: the OTP 23+ handshake between two beamr nodes (name, challenge, reply, ack) | SVG or Mermaid |
| DIST-CONTROL-WIRE-SPEC.md | Diagram: control message encoding on the wire (header, control tuple, payload) | SVG |

## Terminal recording ideas

| Topic | Duration | Audience |
|---|---|---|
| "Hello beamr" -- build from source, compile a Gleam module, run it | ~30s | New contributors |
| "Multi-module project" -- `gleam build` a project with dependencies, run with `--dir` | ~30s | Users evaluating beamr |
| "beamr imports" -- show how to check what a module needs before running it | ~15s | Users debugging missing dependencies |
| "Running the test suite" -- `cargo test --workspace` with output | ~20s | Contributors |

## Video walkthrough ideas

| Topic | Duration | Audience |
|---|---|---|
| "What is beamr?" -- the problem (running Gleam without Erlang), the approach (subset VM), a live demo | 5 min | Developers new to the project |
| "beamr internals tour" -- bytecode loading, term representation, the interpreter loop, scheduling | 10 min | Contributors wanting to understand the VM |
| "Adding a BIF" -- walk through implementing a new native function end to end | 8 min | Contributors |
| "beamr in the Ablative Stack" -- how beamr fits under Aion and Meridian | 5 min | Architects evaluating the stack |

## Tools

- **Terminal recordings**: [asciinema](https://asciinema.org) (renders as text, accessible) or [VHS](https://github.com/charmbracelet/vhs) (GIF/MP4 from a script)
- **Architecture diagrams**: hand-drawn SVG or [Excalidraw](https://excalidraw.com) for the sketch aesthetic
- **Sequence diagrams**: [Mermaid](https://mermaid.js.org) for version-controlled diagrams that render on GitHub
- **Screenshots**: macOS with a clean terminal (ghostty or iTerm2, dark theme matching the Ablative brand)

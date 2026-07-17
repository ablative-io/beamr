# Contributing to beamr

Thank you for your interest in contributing to beamr. This guide covers
the prerequisites, build commands, test workflow, and repository layout
you need to get started.

## Prerequisites

- **Rust** (stable, edition 2024) -- install via [rustup](https://rustup.rs)
- **Gleam** (for compiling test fixtures and example modules) -- install via [gleam.run](https://gleam.run)
- **cargo-clippy** and **rustfmt** -- included with the Rust toolchain

Optional:

- **wasm32-unknown-unknown** target -- `rustup target add wasm32-unknown-unknown` (for the `beamr-wasm` crate)
- **wasm-bindgen-cli** -- `cargo install wasm-bindgen-cli` (for running wasm tests)

## Building

```bash
# Build the entire workspace (debug)
cargo build --workspace

# Build the CLI binary (release)
cargo build --release -p beamr-cli

# Build the core library only
cargo build -p beamr

# Build with specific features
cargo build -p beamr --no-default-features --features cooperative,json

# Check the wasm target compiles
cargo check --target wasm32-unknown-unknown -p beamr --no-default-features --features cooperative,json
```

## Testing

```bash
# Run the full test suite (~1,500+ tests)
cargo test --workspace

# Run tests for a specific crate
cargo test -p beamr
cargo test -p beamr-cli

# Run a single test by name
cargo test -p beamr -- test_name

# Run tests with output visible
cargo test --workspace -- --nocapture
```

## Linting and formatting

Both must pass before committing:

```bash
# Clippy -- treats warnings as errors
cargo clippy --workspace --all-targets -- -D warnings

# Format check (dry run)
cargo fmt --check

# Apply formatting
cargo fmt
```

## Feature flags

The `beamr` crate uses feature flags to control what gets compiled. The
defaults (`std`, `threads`, `net`, `fs`, `jit`, `embedded`, `readiness`)
give you the full VM. Notable non-default features:

| Feature | Purpose |
|---|---|
| `cooperative` | Single-threaded runtime for wasm32 (no OS threads) |
| `json` | OTP 27 `json` module and serde_json bridging |
| `encode` | `.beam` container writer (mirror of the loader) |
| `test-support` | Test utilities exposed for downstream crates |
| `telemetry` | OpenTelemetry-style spans and metrics |

## Repository layout

```
beamr/
  Cargo.toml                 Workspace root
  README.md                  Project overview, CLI usage, architecture
  CONTRIBUTING.md            This file
  LICENSE                    Apache 2.0

  crates/
    beamr/                   Core VM library
      src/
        atom/                Atom table (interned strings, integer lookup)
        capability/          Capability-based security sandbox and audit
        distribution/        Distributed Erlang (handshake, control, remote links)
        etf/                 External Term Format encode/decode
        ets/                 ETS in-memory tables (set, ordered_set, bag, match specs)
        gc/                  Generational copying garbage collector
        interpreter/         Bytecode interpreter, opcode dispatch, pattern matching
        io/                  I/O backend (io_uring on Linux, portable fallback)
        jit/                 Cranelift-backed JIT, AOT cache, profiler, safepoints
        loader/              .beam file parser, decoder, module loader
        mailbox/             Lock-free process mailboxes with selective receive
        native/              200+ BIF implementations:
          bifs               Core erlang BIFs (arithmetic, comparison, types)
          gate3_bifs         Extended erlang BIFs (conversion, bitwise, math)
          gleam_ffi          Gleam-specific FFI functions
          otp_stubs          OTP module stubs (gleam_erlang, gleam_otp)
          stdlib_stubs       Stdlib BIFs (collections, strings, IO, encoding)
          process_bifs       Process management BIFs (spawn, link, monitor)
        process/             Process state, heap, stack, registry
        replay/              Deterministic record/replay and step debugger
        scheduler/           Preemptive scheduler with work-stealing
        supervision/         OTP-style links, monitors, exit signal propagation
        telemetry/           OpenTelemetry spans, metrics, lifecycle events
        term/                Tagged term representation (64-bit tagged pointers)
      tests/                 Integration tests

    beamr-cli/               Command-line .beam runner (the `beamr` binary)
    beamr-wasm/              WebAssembly target (cooperative single-threaded runtime)
    gleam-types/             Gleam type representations shared across crates

  docs/
    adr/                     Architecture Decision Records (ADR 001-012)
    design/                  Design documents
    files/                   Supporting files for documentation
    *.md                     Specifications and design documents
```

## Architecture Decision Records

Design decisions are documented as ADRs in `docs/adr/`. Read these before
making changes to the areas they cover -- they explain the reasoning behind
key architectural choices. See the table in the README for a summary.

## Workflow

1. Fork the repository and create a feature branch
2. Make your changes
3. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings`
4. Run `cargo test --workspace` and confirm all tests pass
5. Open a pull request against `main`

## Adding a new BIF

The most common contribution is implementing a new native BIF. The pattern:

1. Find the BIF's spec in the Erlang/OTP documentation
2. Add the implementation in the appropriate module under `crates/beamr/src/native/`
3. Register it in the BIF dispatch table
4. Add tests in `crates/beamr/tests/` or as unit tests alongside the implementation
5. Run `cargo test --workspace` to verify

## License

By contributing, you agree that your contributions will be licensed under the
Apache License, Version 2.0, the same license as the project.

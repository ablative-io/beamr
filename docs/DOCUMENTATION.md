# Beamr — Documentation

## What is Beamr?

Beamr is a runtime — the engine that actually runs your code. Think of it like the engine in a car: you don't interact with it directly, but nothing moves without it.

Specifically, Beamr runs programs written in a language called Gleam. It's built from the ground up in Rust, which means it's fast, reliable, and doesn't need any other runtime installed on your machine.

## Why does Beamr exist?

Most systems that run Gleam code rely on the Erlang virtual machine (called the BEAM), which was originally built for telephone switches in the 1980s. It's battle-tested but comes with a lot of baggage — you need to install Erlang, manage its dependencies, and accept its constraints.

Beamr replaces that. It runs the same Gleam code but on a purpose-built engine that:

- **Starts faster** — no Erlang boot sequence
- **Runs in the browser** — compiles to WebAssembly so your code can run on any device
- **Handles crashes gracefully** — if one part of your program fails, only that part restarts while everything else keeps running
- **Gets faster over time** — a built-in compiler watches which parts of your code run most often and optimises them automatically

## How does Beamr fit in the Ablative Stack?

Beamr is the foundation layer. Every other part of the stack runs on top of it:

```
You write code in Gleam
        ↓
Gleam compiles to bytecode
        ↓
Beamr runs the bytecode      ← this is Beamr
        ↓
Your application works
```

Haematite (storage), Liminal (messaging), and Aion (workflows) all run on Beamr. If you're using any part of the Ablative Stack, Beamr is running underneath.

## Current Status

**Version 0.x** — Beamr is in active development and used internally by Ablative. It has over 1,500 tests and 117,000 lines of code. Core features (running Gleam code, crash recovery, scheduling, WebAssembly) are working. Performance optimisation and browser support are ongoing.

## Getting Started

### What you'll need

- **Rust** — Beamr is written in Rust, so you'll need the Rust toolchain installed. If you don't have it, go to [rustup.rs](https://rustup.rs) and follow the instructions. It takes about 2 minutes.
- **Gleam** — You'll write your programs in Gleam. Install it from [gleam.run](https://gleam.run). Again, about 2 minutes.

### Install Beamr

Open your terminal and run:

```
cargo install beamr-cli --locked
```

This downloads and builds Beamr. It might take a few minutes the first time (Rust compiles everything from source). When it's done, you'll have a `beamr` command available.

### Run your first program

1. Create a new Gleam project:

```
gleam new hello
cd hello
```

2. Build it:

```
gleam build
```

3. Run it with Beamr:

```
beamr build/dev/erlang/hello/ebin/hello.beam
```

That's it — your Gleam code is running on Beamr.

### Use Beamr as a library

If you're building a Rust application that needs to run Gleam code, add Beamr as a dependency:

```toml
[dependencies]
beamr = "0.x"
```

```rust
use beamr::vm::Vm;

let vm = Vm::new();
// Load and run Gleam bytecode
```

## Key Concepts

**Processes** — Beamr runs your code in lightweight processes (not operating system processes — these are much smaller and faster). You can have millions of them. Each one is isolated, so if one crashes, the others keep running.

**Supervision** — Processes are organised into supervision trees. A supervisor watches its child processes and restarts them if they fail. This is why Beamr applications self-heal.

**Scheduling** — Beamr automatically distributes work across your computer's CPU cores. You don't need to think about threads or parallelism — it handles that for you.

**Hot code loading** — You can update your code while the application is running, without dropping connections or losing state. Like changing the tyres on a moving car.

## Learn More

- [Gleam language guide](https://gleam.run) — Learn the language that runs on Beamr
- [Ablative Stack overview](https://ablative.dev) — See how Beamr fits with the other components

## License

Apache-2.0 — free to use, modify, and distribute, including in commercial projects.

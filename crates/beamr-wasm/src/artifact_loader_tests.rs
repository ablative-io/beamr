//! WPORT-6 R3: the artifact-loader wall battery — the repo's FIRST wasm-side
//! executions of the real `.beam` decode path and FIRST real-VM
//! JS-orchestration tests (at the brief's pin no wasm test exercised
//! `load_module`; the decoder was bypassed entirely on wasm — ground pack §6).
//! Every wall drives the loader through the exported `load_artifacts` surface
//! with an injected fetch double; no wall pumps manually and no wall polls —
//! completion re-enters via each fetch Promise's own microtask continuation.

use std::cell::RefCell;
use std::rc::Rc;

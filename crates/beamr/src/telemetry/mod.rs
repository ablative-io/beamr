//! Optional OpenTelemetry integration for Beamr runtime events.
//!
//! This module is compiled only with the `telemetry` feature so default builds
//! do not carry OpenTelemetry dependencies or call-site overhead.

pub mod lifecycle;
pub mod metrics;
pub mod spans;

pub use metrics::{
    record_workflow_finished, record_workflow_started, record_workflow_step_completed,
};
pub use spans::{
    ProcessTraceContext, TraceCarrier, extract_context, inject_context, inject_current_context,
};

#[must_use]
pub fn current_trace_context() -> TraceCarrier {
    inject_current_context()
}

//! Telemetry a process carries: when its receive wait started, and its
//! trace context.

use super::Process;

impl Process {
    #[cfg(feature = "telemetry")]
    pub(crate) fn mark_receive_wait_started(&mut self) {
        if self.receive_wait_started.is_none() {
            self.receive_wait_started = Some(crate::telemetry::spans::receive_wait_started_now());
        }
    }

    #[cfg(feature = "telemetry")]
    pub(crate) fn take_receive_wait_duration(&mut self) -> Option<std::time::Duration> {
        self.receive_wait_started
            .take()
            .map(|started| started.elapsed())
    }

    #[cfg(feature = "telemetry")]
    pub(crate) fn set_trace_context(
        &mut self,
        trace_context: Option<crate::telemetry::spans::ProcessTraceContext>,
    ) {
        self.trace_context = trace_context;
    }

    #[cfg(feature = "telemetry")]
    pub(crate) const fn trace_context(
        &self,
    ) -> Option<&crate::telemetry::spans::ProcessTraceContext> {
        self.trace_context.as_ref()
    }
}

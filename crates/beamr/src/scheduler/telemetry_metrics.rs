//! Scheduler and process metrics recorded for telemetry.

use super::{SharedState, lock_or_recover};
use crate::process::Process;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
#[cfg(all(feature = "threads", feature = "telemetry"))]
use std::time::Instant;

#[cfg(feature = "telemetry")]
pub(super) struct TelemetryMetricState {
    sample_interval: Duration,
    pub(super) last_process_samples: Mutex<std::collections::HashMap<u64, Instant>>,
    /// When the heap total was last summed, and what it came to. Summing
    /// visits every process's slot, so it runs once per `sample_interval`
    /// rather than on every slice and exit: on every exit it made a burst
    /// of n exits cost n² (20,000 exits took 55 s).
    pub(super) heap_words_sample: Mutex<Option<(Instant, usize)>>,
    scheduler_executing_nanos: AtomicU64,
    scheduler_idle_nanos: AtomicU64,
}

#[cfg(feature = "telemetry")]
impl TelemetryMetricState {
    pub(super) fn new(sample_interval: Duration) -> Self {
        Self {
            sample_interval,
            last_process_samples: Mutex::new(std::collections::HashMap::new()),
            heap_words_sample: Mutex::new(None),
            scheduler_executing_nanos: AtomicU64::new(0),
            scheduler_idle_nanos: AtomicU64::new(0),
        }
    }
}

impl SharedState {
    #[cfg(feature = "telemetry")]
    pub(super) fn record_scheduler_executing(&self, duration: Duration) {
        self.add_scheduler_duration(&self.telemetry_metrics.scheduler_executing_nanos, duration);
        self.record_vm_health_metrics();
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_scheduler_idle(&self, duration: Duration) {
        self.add_scheduler_duration(&self.telemetry_metrics.scheduler_idle_nanos, duration);
        self.record_vm_health_metrics();
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_process_slice_metrics(&self, process: &Process, reductions_consumed: u32) {
        let now = Instant::now();
        {
            let mut last_samples = lock_or_recover(&self.telemetry_metrics.last_process_samples);
            if let Some(last_sample) = last_samples.get(&process.pid())
                && now.duration_since(*last_sample) < self.telemetry_metrics.sample_interval
            {
                return;
            }
            last_samples.insert(process.pid(), now);
        }
        crate::telemetry::metrics::record_process_slice(
            process.pid(),
            reductions_consumed,
            process.mailbox().message_count(),
        );
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn remove_process_metric_state(&self, pid: u64) {
        lock_or_recover(&self.telemetry_metrics.last_process_samples).remove(&pid);
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_vm_health_metrics(&self) {
        let processes_alive = self.process_count();
        let heap_words = self.sampled_heap_words(processes_alive);
        crate::telemetry::metrics::record_vm_health(
            processes_alive,
            heap_words,
            self.scheduler_utilization(),
        );
        let compile_outcomes = self.jit_profiler.compile_outcome_counters();
        crate::telemetry::metrics::record_jit_compile_outcomes(
            compile_outcomes.submissions,
            compile_outcomes.successes,
            compile_outcomes.unsupported,
            compile_outcomes.transient_failures,
        );
    }

    /// The heap total across processes, summed at most once per sample
    /// interval. With no process alive it is zero without a sum, so the
    /// gauge reads true the moment the last process goes.
    #[cfg(feature = "telemetry")]
    fn sampled_heap_words(&self, processes_alive: usize) -> usize {
        if processes_alive == 0 {
            return 0;
        }
        let now = Instant::now();
        let mut sample = lock_or_recover(&self.telemetry_metrics.heap_words_sample);
        if let Some((taken, words)) = *sample
            && now.duration_since(taken) < self.telemetry_metrics.sample_interval
        {
            return words;
        }
        let (words, _) = self.process_heap_and_binary_words();
        *sample = Some((now, words));
        words
    }

    #[cfg(feature = "telemetry")]
    fn scheduler_utilization(&self) -> f64 {
        let executing = self
            .telemetry_metrics
            .scheduler_executing_nanos
            .load(Ordering::Relaxed);
        let idle = self
            .telemetry_metrics
            .scheduler_idle_nanos
            .load(Ordering::Relaxed);
        let total = executing.saturating_add(idle);
        if total == 0 {
            0.0
        } else {
            executing as f64 / total as f64
        }
    }

    #[cfg(feature = "telemetry")]
    fn add_scheduler_duration(&self, counter: &AtomicU64, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        let _previous = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(nanos))
        });
    }
}

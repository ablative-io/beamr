//! Throwaway idle-burn instrument (Artemis, aion#71 gauge): builds N schedulers
//! with the stated worker count and full runtime services, then idles so an
//! outer shell can sample %CPU from outside.
use beamr::atom::AtomTable;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices};
use std::sync::Arc;

fn main() {
    let workers: Option<usize> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    let instances: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut schedulers = Vec::new();
    for _ in 0..instances {
        let config = SchedulerConfig {
            thread_count: workers,
            ..SchedulerConfig::default()
        };
        schedulers.push(
            Scheduler::with_services_and_code_server(
                config,
                SchedulerServices::full_runtime(),
                Arc::new(ModuleRegistry::new()),
                Arc::new(AtomTable::with_common_atoms()),
                Arc::new(BifRegistryImpl::new()),
            )
            .expect("scheduler"),
        );
    }
    let per = schedulers.first().map(|s| s.worker_names().len()).unwrap_or(0);
    println!("READY instances={instances} workers_each={per}");
    std::thread::sleep(std::time::Duration::from_secs(60));
}

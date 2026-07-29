/// WALL 1b — ordering tripwire (EXIT-001; ruled addition, Artemis 17:19Z,
/// discharging the STRICT reading of Hermes's rider; committed Wall 1
/// untouched). Wall 1 certifies the value-at-wake face and is
/// deterministically green under a publish-before-install mutation:
/// `terminate_process` runs finalization synchronously on the calling
/// thread, so the watcher cannot observe any intermediate state — there is
/// no window, which means Wall 1 cannot fail at the ORDERING face. This
/// tripwire pins that face deterministically instead: the killer runs on
/// its own thread, the test-only post-send publication gate parks it
/// inside the exit publication, and at the park the outcome — WITH its
/// pre-kill enqueued value — must already be takeable while the watch has
/// not yet fired. Under publish-before-install the park happens before the
/// install and the at-park take fails cold: red, not a race.
#[test]
fn wall1b_ordering_tripwire_outcome_installed_before_exit_publication_at_scheduler_seam() {
    let scheduler = test_scheduler(1);
    let pid = scheduler
        .spawn_native(Box::new(|| Box::new(WaitForTermination)))
        .expect("native process spawns");

    let watch = match scheduler.watch_exit(pid) {
        ExitWatchState::Live(watch) => watch,
        other => panic!("precondition: live process arms a watch, got {other:?}"),
    };

    // The reply is enqueued STRICTLY BEFORE the kill, value face riding along.
    scheduler
        .shared
        .exit_results
        .insert(pid, OwnedTerm::immediate(Term::small_int(4321)));

    let observer = scheduler.shared.exit_tombstones.install_event_publication_gate();
    std::thread::scope(|scope| {
        let killer = scope.spawn(|| scheduler.terminate_process(pid, ExitReason::Kill));

        // The killer parks inside the exit publication (post-send gate)…
        observer.wait_for_publication(EVENT_TIMEOUT);

        // …and AT THE PARK the outcome is already installed with its value:
        // outcomes-before-exits, asserted where a reordering cannot hide.
        let (reason, value) = scheduler.take_exit_outcome(pid).expect(
            "outcome must be takeable at the publication park — the \
             outcomes-before-exits pin Hermes's conclusion logic rests on",
        );
        assert_eq!(reason, ExitReason::Kill);
        assert_eq!(
            value.root().as_small_int(),
            Some(4321),
            "the pre-kill enqueued VALUE must be observable at the park"
        );

        // The watch fire comes strictly AFTER the existing publication.
        assert_eq!(
            watch.recv_timeout(Duration::ZERO),
            Err(ExitEventRecvError::Timeout),
            "watch must not fire before the exit publication returns"
        );

        observer.release_publication(EVENT_TIMEOUT);
        killer.join().expect("terminal caller completes");
        scheduler.shared.exit_tombstones.clear_event_publication_gate();

        // The wake completes once publication (and the appended fire) finish.
        assert_eq!(
            watch.recv_timeout(EVENT_TIMEOUT),
            Ok((pid, ExitReason::Kill)),
            "the waiter wakes after release"
        );
    });
    scheduler.shutdown();
}

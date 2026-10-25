use rusty_tokio::time;
use rusty_tokio::Builder;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn advance_fires_a_sleep_without_any_real_waiting() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let started = std::time::Instant::now();

        let (tx, rx) = rusty_tokio::sync::oneshot::channel::<()>();
        rusty_tokio::spawn(async move {
            time::sleep(Duration::from_secs(3600)).await;
            let _ = tx.send(());
        });
        // Give the spawned task a chance to actually run and register
        // its sleep with the timer driver before advancing.
        rusty_tokio::task::yield_now().await;

        time::advance(Duration::from_secs(3600)).await;
        rx.await.unwrap();

        // No real waiting happened -- the whole test should take
        // milliseconds, not anywhere near an hour.
        assert!(started.elapsed() < Duration::from_secs(5));
    });
}

#[test]
fn advance_fires_several_sleeps_in_deadline_order() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        for (label, delay) in [("c", 30), ("a", 10), ("b", 20)] {
            let order = order.clone();
            rusty_tokio::spawn(async move {
                time::sleep(Duration::from_secs(delay)).await;
                order.lock().unwrap().push(label);
            });
        }
        // Give every spawned task a chance to actually run once and
        // register its sleep with the timer driver before advancing --
        // otherwise `advance` would find an empty heap and have nothing
        // to fire.
        rusty_tokio::task::yield_now().await;

        time::advance(Duration::from_secs(30)).await;
        assert_eq!(*order.lock().unwrap(), vec!["a", "b", "c"]);
    });
}

#[test]
fn advance_lets_a_chained_sleep_fire_within_the_same_call() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let count = Arc::new(AtomicUsize::new(0));
        let count2 = count.clone();

        rusty_tokio::spawn(async move {
            time::sleep(Duration::from_secs(10)).await;
            count2.fetch_add(1, Ordering::SeqCst);
            time::sleep(Duration::from_secs(10)).await;
            count2.fetch_add(1, Ordering::SeqCst);
        });
        rusty_tokio::task::yield_now().await;

        time::advance(Duration::from_secs(25)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn resume_makes_sleep_track_real_time_again() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        time::advance(Duration::from_secs(100)).await;
        time::resume();

        let started = std::time::Instant::now();
        time::sleep(Duration::from_millis(20)).await;
        assert!(started.elapsed() >= Duration::from_millis(20));
    });
}

#[test]
#[should_panic(expected = "current-thread runtime flavor")]
fn pause_panics_on_a_multi_threaded_runtime() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    rt.block_on(async {
        time::pause();
    });
}

#[test]
#[should_panic(expected = "already paused")]
fn pause_twice_panics() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        time::pause();
    });
}

#[test]
#[should_panic(expected = "time is not paused")]
fn resume_without_pause_panics() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::resume();
    });
}

#[test]
#[should_panic(expected = "requires time::pause() first")]
fn advance_without_pause_panics() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::advance(Duration::from_secs(1)).await;
    });
}

#[test]
fn advance_drives_an_interval_through_several_ticks_deterministically() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut interval = time::interval(Duration::from_secs(5));

        time::advance(Duration::from_secs(5)).await;
        let first = interval.tick().await;

        time::advance(Duration::from_secs(15)).await;
        let second = interval.tick().await;
        let third = interval.tick().await;
        let fourth = interval.tick().await;

        assert_eq!(second, first + Duration::from_secs(5));
        assert_eq!(third, first + Duration::from_secs(10));
        assert_eq!(fourth, first + Duration::from_secs(15));
    });
}

#[test]
fn sleep_reset_re_arms_to_a_new_deadline_without_firing_the_old_one() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut sleeper = time::sleep(Duration::from_secs(5));
        // Derived from the sleeper's own (paused-clock) deadline rather
        // than a fresh `std::time::Instant::now()` read, which would
        // drift from the virtual clock -- see `advance_*` tests above
        // for the same reasoning.
        let new_deadline = sleeper.deadline() + Duration::from_secs(15);

        // Advance past the *original* deadline without polling `sleeper`
        // in between, then reset it further out before it's ever
        // actually awaited -- the old deadline must not have "stuck".
        time::advance(Duration::from_secs(3)).await;
        sleeper.reset(new_deadline);
        assert_eq!(sleeper.deadline(), new_deadline);

        time::advance(Duration::from_secs(17)).await;
        sleeper.await;
        // If reset hadn't taken effect, this would have already resolved
        // back when time passed the original 5s deadline, well before
        // this point -- reaching here at all is most of the assertion.
    });
}

#[test]
fn sleep_reset_after_a_poll_cancels_the_stale_registration_and_rearms() {
    use std::future::Future;
    use std::pin::Pin;

    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut sleeper = time::sleep(Duration::from_secs(5));
        let original_deadline = sleeper.deadline();

        // Poll once directly (registers with the timer driver, since 5s
        // hasn't elapsed yet) without going through a real `.await`.
        std::future::poll_fn(|cx| {
            let _ = Pin::new(&mut sleeper).poll(cx);
            std::task::Poll::Ready(())
        })
        .await;

        // Reset to fire 4s earlier than the original schedule --
        // derived from `original_deadline` (the crate's own paused
        // clock), not a fresh `Instant::now()` read.
        let new_deadline = original_deadline - Duration::from_secs(4);
        sleeper.reset(new_deadline);

        time::advance(Duration::from_secs(1)).await;
        sleeper.await;
    });
}

#[test]
fn sleep_is_elapsed_is_false_before_the_deadline_and_true_after() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let sleeper = time::sleep(Duration::from_secs(5));
        assert!(!sleeper.is_elapsed());

        time::advance(Duration::from_secs(3)).await;
        assert!(!sleeper.is_elapsed());

        time::advance(Duration::from_secs(2)).await;
        assert!(sleeper.is_elapsed());
    });
}

#[test]
fn interval_period_reports_the_configured_period() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        let interval = time::interval(Duration::from_secs(5));
        assert_eq!(interval.period(), Duration::from_secs(5));
    });
}

#[test]
fn interval_reset_reschedules_one_period_from_now() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut interval = time::interval(Duration::from_secs(5));
        time::advance(Duration::from_secs(5)).await;
        let first = interval.tick().await;

        // Let some of the *next* scheduled period elapse, then reset --
        // the next tick should measure a fresh period from the reset
        // point, not from `first`'s original schedule.
        time::advance(Duration::from_secs(2)).await;
        interval.reset();
        time::advance(Duration::from_secs(5)).await;
        let second = interval.tick().await;

        assert_eq!(second, first + Duration::from_secs(7));
    });
}

#[test]
fn interval_reset_immediately_fires_the_next_tick_right_away() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut interval = time::interval(Duration::from_secs(5));
        time::advance(Duration::from_secs(5)).await;
        let first = interval.tick().await;

        time::advance(Duration::from_secs(1)).await;
        interval.reset_immediately();
        let second = interval.tick().await;
        assert_eq!(second, first + Duration::from_secs(1));
    });
}

#[test]
fn interval_reset_after_reschedules_from_now_by_the_given_duration() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut interval = time::interval(Duration::from_secs(5));
        time::advance(Duration::from_secs(5)).await;
        let first = interval.tick().await;

        time::advance(Duration::from_secs(1)).await;
        interval.reset_after(Duration::from_secs(3));
        time::advance(Duration::from_secs(3)).await;
        let second = interval.tick().await;
        assert_eq!(second, first + Duration::from_secs(4));
    });
}

#[test]
fn interval_reset_at_reschedules_to_the_given_absolute_deadline() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let mut interval = time::interval(Duration::from_secs(5));
        time::advance(Duration::from_secs(5)).await;
        let first = interval.tick().await;

        // Derived from `first` (the crate's own clock) rather than a
        // fresh `std::time::Instant::now()` -- while paused, the crate's
        // virtual clock only moves via `advance`, so reading real wall
        // time here would drift from it.
        let deadline = first + Duration::from_secs(10);
        interval.reset_at(deadline);
        time::advance(Duration::from_secs(10)).await;
        let second = interval.tick().await;
        assert_eq!(second, deadline);
    });
}

// ---- `time::Instant` (#898, gap G3) and auto-advance ----

#[test]
fn instant_now_moves_by_exactly_the_advanced_duration() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let before = time::Instant::now();
        time::advance(Duration::from_secs(90)).await;
        assert_eq!(time::Instant::now() - before, Duration::from_secs(90));
        assert_eq!(before.elapsed(), Duration::from_secs(90));
    });
}

#[test]
fn instant_now_is_frozen_while_paused() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let first = time::Instant::now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(time::Instant::now(), first);
    });
}

#[test]
fn instant_now_outside_a_runtime_reads_real_time() {
    let before = std::time::Instant::now();
    let now = time::Instant::now().into_std();
    assert!(now >= before);
    assert!(now <= std::time::Instant::now());
}

#[test]
fn sleep_deadline_and_interval_ticks_are_on_the_runtime_clock() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let start = time::Instant::now();
        let sleeper = time::sleep(Duration::from_secs(5));
        assert_eq!(sleeper.deadline(), start + Duration::from_secs(5));

        let mut ticker = time::interval_at(start + Duration::from_secs(1), Duration::from_secs(1));
        let first = ticker.tick().await;
        assert_eq!(first, start + Duration::from_secs(1));
        assert_eq!(time::Instant::now(), first);
    });
}

#[test]
fn instant_arithmetic_matches_tokio() {
    let a = time::Instant::now();
    let b = a + Duration::from_secs(2);
    assert_eq!(b - a, Duration::from_secs(2));
    // `Sub<Instant>` and `duration_since` saturate, as tokio's do.
    assert_eq!(a - b, Duration::ZERO);
    assert_eq!(a.duration_since(b), Duration::ZERO);
    assert_eq!(a.saturating_duration_since(b), Duration::ZERO);
    assert_eq!(a.checked_duration_since(b), None);
    assert_eq!(b.checked_duration_since(a), Some(Duration::from_secs(2)));
    assert_eq!(a.checked_add(Duration::from_secs(2)), Some(b));
    assert_eq!(b.checked_sub(Duration::from_secs(2)), Some(a));
    assert_eq!(b - Duration::from_secs(2), a);

    let mut c = a;
    c += Duration::from_secs(3);
    c -= Duration::from_secs(1);
    assert_eq!(c, b);
    assert!(a < b);

    let std_a: std::time::Instant = a.into();
    assert_eq!(time::Instant::from_std(std_a), a);
    assert_eq!(time::Instant::from(a.into_std()), a);
    assert_eq!(format!("{a:?}"), format!("{std_a:?}"));
}

#[test]
fn paused_clock_auto_advances_a_long_sleep_when_idle() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let wall = std::time::Instant::now();
        let start = time::Instant::now();
        time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(3600));
        assert!(wall.elapsed() < Duration::from_secs(5));
    });
}

#[test]
fn auto_advance_fires_spawned_sleeps_in_deadline_order() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let start = time::Instant::now();
        let (tx, mut rx) = rusty_tokio::sync::mpsc::unbounded_channel();
        for secs in [30u64, 10, 20] {
            let tx = tx.clone();
            rusty_tokio::spawn(async move {
                time::sleep(Duration::from_secs(secs)).await;
                let _ = tx.send((secs, time::Instant::now()));
            });
        }
        drop(tx);
        let mut seen = Vec::new();
        while let Some((secs, at)) = rx.recv().await {
            assert_eq!(at - start, Duration::from_secs(secs));
            seen.push(secs);
        }
        assert_eq!(seen, [10, 20, 30]);
    });
}

#[test]
fn auto_advance_waits_for_in_flight_spawn_blocking() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        // Without the hold-off, the idle scheduler would jump straight
        // past the 1s timeout while the blocking job still runs.
        let job = rusty_tokio::spawn_blocking(|| {
            std::thread::sleep(Duration::from_millis(100));
            7
        });
        let result = time::timeout(Duration::from_secs(1), job).await;
        assert_eq!(result.unwrap().unwrap(), 7);
    });
}

#[test]
fn auto_advance_skips_timers_whose_sleep_was_dropped() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        time::pause();
        let start = time::Instant::now();
        // The timeout's inner future wins; its 10s `Sleep` is dropped
        // but its heap entry stays until reached.
        let won = time::timeout(Duration::from_secs(10), async { 1 }).await;
        assert_eq!(won, Ok(1));
        time::sleep(Duration::from_secs(1)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    });
}

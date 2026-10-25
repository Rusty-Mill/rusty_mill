//! Round-robin event selection for the runtime's event loops.
//!
//! A `select!` that polls its arms in written order (`biased;`) lets an
//! always-ready earlier arm (a flooded output channel, a `Notify` that keeps
//! collecting permits) starve every later one, and a randomized `select!`
//! only bounds that probabilistically. [`fair_select`] rotates the first arm
//! polled on each call instead, so with `n` arms every ready arm wins within
//! `n` consecutive calls.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

/// Await the first of `arms` to complete. Polling starts at arm `*turn % n`,
/// and `*turn` is advanced on every call. The futures in the arms that did
/// not win are left un-completed. As with `select!`, each arm must be
/// cancel-safe if the caller drops it and builds it again on the next call.
/// Arms are `Send` so a spawned task (a web-bridge session) can await them.
///
/// # Panics
/// Panics if `arms` is empty.
pub(crate) async fn fair_select<W>(
    turn: &mut usize,
    arms: &mut [Pin<&mut (dyn Future<Output = W> + Send)>],
) -> W {
    let n = arms.len();
    assert!(n > 0, "fair_select needs at least one arm");
    let start = *turn % n;
    *turn = turn.wrapping_add(1);
    poll_fn(|cx| {
        for i in 0..n {
            if let Poll::Ready(w) = arms[(start + i) % n].as_mut().poll(cx) {
                return Poll::Ready(w);
            }
        }
        Poll::Pending
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::sync::{Notify, mpsc};
    use std::pin::pin;
    use std::time::Duration;

    /// Every arm is always ready: each one wins exactly once per `n` calls.
    #[rusty_tokio::test]
    async fn always_ready_arms_take_turns() {
        let mut turn = 0;
        let mut wins = [0usize; 3];
        for _ in 0..30 {
            let (a, b, c) = (pin!(async { 0 }), pin!(async { 1 }), pin!(async { 2 }));
            wins[fair_select(&mut turn, &mut [a, b, c]).await] += 1;
        }
        assert_eq!(wins, [10, 10, 10]);
    }

    #[derive(Debug, PartialEq)]
    enum Ev {
        Output(Option<u32>),
        Frame,
        Resize,
    }

    /// The Windows render loop's shape under an output flood: the channel is
    /// never empty and every chunk re-notifies the frame. A frame (repaint, or
    /// a shutdown check) and a ready resize tick still make progress within
    /// one round of turns.
    #[rusty_tokio::test]
    async fn output_flood_does_not_starve_frame_or_resize() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for i in 0..1_000 {
            tx.send(i).unwrap();
        }
        let frame = Notify::new();
        frame.notify_one();
        let mut resize = rusty_tokio::time::interval(Duration::from_millis(1));
        // Let the first resize tick fall due, so it is ready like the rest.
        rusty_tokio::time::sleep(Duration::from_millis(5)).await;
        let mut notified = pin!(frame.notified());
        let mut turn = 0;
        let mut seen = Vec::new();
        for _ in 0..6 {
            let ev = {
                let out = pin!(async { Ev::Output(rx.recv().await) });
                let fr = pin!(async {
                    (&mut notified).await;
                    Ev::Frame
                });
                let rs = pin!(async {
                    resize.tick().await;
                    Ev::Resize
                });
                fair_select(&mut turn, &mut [out, fr, rs]).await
            };
            if ev == Ev::Frame {
                notified.set(frame.notified());
            }
            if matches!(ev, Ev::Output(_)) {
                frame.notify_one(); // every chunk asks for a repaint
            }
            seen.push(ev);
        }
        assert!(seen.contains(&Ev::Frame), "{seen:?}");
        assert!(seen.contains(&Ev::Resize), "{seen:?}");
        assert!(seen.iter().filter(|e| matches!(e, Ev::Output(_))).count() >= 2);
    }

    /// The Unix render loop's shape: the frame `Notify` is re-armed after
    /// every wake and re-notified each time (sustained output), and a resize
    /// is pending. The resize is served on the next turn.
    #[rusty_tokio::test]
    async fn repaint_flood_does_not_starve_resize() {
        let frame = Notify::new();
        let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<()>();
        resize_tx.send(()).unwrap();
        let mut notified = pin!(frame.notified());
        let mut turn = 0;
        let mut seen = Vec::new();
        for _ in 0..2 {
            frame.notify_one();
            let ev = {
                let fr = pin!(async {
                    (&mut notified).await;
                    Ev::Frame
                });
                let rs = pin!(async {
                    resize_rx.recv().await;
                    Ev::Resize
                });
                fair_select(&mut turn, &mut [fr, rs]).await
            };
            if ev == Ev::Frame {
                notified.set(frame.notified());
            }
            seen.push(ev);
        }
        assert_eq!(seen, [Ev::Frame, Ev::Resize]);
    }
}

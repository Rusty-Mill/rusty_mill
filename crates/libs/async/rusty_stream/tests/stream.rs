use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusty_stream::channel::mpsc;
use rusty_stream::future::{join_all, ready, BoxFuture};
use rusty_stream::stream::{iter, once, select_all, unfold};
use rusty_stream::{try_stream, BoxStream, Stream, StreamExt, TryStreamExt};

#[rusty_tokio::test]
async fn next_map_take_collect() {
    let mut s = iter(1..=5).map(|n| n * 10).take(3);
    assert_eq!(s.next().await, Some(10));
    let rest: Vec<i32> = s.collect().await;
    assert_eq!(rest, vec![20, 30]);
}

#[rusty_tokio::test]
async fn take_zero_is_empty() {
    let items: Vec<i32> = iter(1..4).take(0).collect().await;
    assert!(items.is_empty());
}

#[rusty_tokio::test]
async fn filter_filter_map_then() {
    let evens: Vec<i32> = iter(1..=6).filter(|n| ready(n % 2 == 0)).collect().await;
    assert_eq!(evens, vec![2, 4, 6]);

    let halves: Vec<i32> = iter(1..=6)
        .filter_map(|n| async move { (n % 2 == 0).then_some(n / 2) })
        .collect()
        .await;
    assert_eq!(halves, vec![1, 2, 3]);

    let doubled: Vec<i32> = iter(1..=3).then(|n| async move { n * 2 }).collect().await;
    assert_eq!(doubled, vec![2, 4, 6]);
}

#[rusty_tokio::test]
async fn buffered_keeps_order_and_limit() {
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let delays = [30u64, 10, 20, 5, 15];
    let futures = delays.into_iter().enumerate().map(|(i, ms)| {
        let running = Arc::clone(&running);
        let peak = Arc::clone(&peak);
        async move {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            rusty_tokio::time::sleep(Duration::from_millis(ms)).await;
            running.fetch_sub(1, Ordering::SeqCst);
            i
        }
    });
    let order: Vec<usize> = iter(futures).buffered(2).collect().await;
    assert_eq!(order, vec![0, 1, 2, 3, 4]);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[rusty_tokio::test]
async fn try_adapters() {
    let s = iter(vec![Ok::<i32, String>(1), Ok(2)]);
    let doubled: Result<Vec<i32>, String> = s.map_ok(|n| n * 2).try_collect().await;
    assert_eq!(doubled, Ok(vec![2, 4]));

    let s = iter(vec![Ok(1), Err("boom"), Ok(3)]);
    let first_error: Result<Vec<i32>, usize> = s.map_err(str::len).try_collect().await;
    assert_eq!(first_error, Err(4));

    let mut s = iter(vec![Ok::<i32, ()>(7)]);
    assert_eq!(s.try_next().await, Ok(Some(7)));
    assert_eq!(s.try_next().await, Ok(None));
}

#[rusty_tokio::test]
async fn once_unfold_join_all() {
    let one: Vec<i32> = once(async { 9 }).collect().await;
    assert_eq!(one, vec![9]);

    let countdown: Vec<u32> = unfold(3u32, |n| async move { (n > 0).then(|| (n, n - 1)) })
        .collect()
        .await;
    assert_eq!(countdown, vec![3, 2, 1]);

    let outputs = join_all((0..3u64).map(|i| async move {
        rusty_tokio::time::sleep(Duration::from_millis(30 - i * 10)).await;
        i
    }))
    .await;
    assert_eq!(outputs, vec![0, 1, 2]);
    assert!(join_all(Vec::<BoxFuture<'static, ()>>::new())
        .await
        .is_empty());
}

#[rusty_tokio::test]
async fn select_all_drains_every_stream() {
    let merged = select_all(vec![iter(vec![1, 2]), iter(vec![]), iter(vec![3])]);
    let mut items: Vec<i32> = merged.collect().await;
    items.sort_unstable();
    assert_eq!(items, vec![1, 2, 3]);
}

#[rusty_tokio::test]
async fn select_all_alternates_between_ready_streams() {
    let merged = select_all(vec![iter(vec!['a', 'a']), iter(vec!['b', 'b'])]);
    let items: Vec<char> = merged.collect().await;
    assert_eq!(items, vec!['a', 'b', 'a', 'b']);
}

#[rusty_tokio::test]
async fn unbounded_channel() {
    let (tx, mut rx) = mpsc::unbounded();
    let tx2 = tx.clone();
    rusty_tokio::spawn(async move {
        tx.unbounded_send(1).unwrap();
        tx2.unbounded_send(2).unwrap();
    });
    assert_eq!(rx.next().await, Some(1));
    assert_eq!(rx.next().await, Some(2));
    // Both senders are gone, so the stream ends.
    assert_eq!(rx.next().await, None);
}

#[test]
fn unbounded_send_after_receiver_drop_fails() {
    let (tx, rx) = mpsc::unbounded::<u8>();
    drop(rx);
    assert!(tx.is_closed());
    assert_eq!(tx.unbounded_send(5).unwrap_err().into_inner(), 5);
}

#[rusty_tokio::test]
async fn stream_macro_yields_in_order() {
    let s = rusty_stream::stream! {
        for i in 0..3 {
            yield i;
        }
        let tail = async { 10 }.await;
        yield tail;
    };
    let items: Vec<i32> = s.collect().await;
    assert_eq!(items, vec![0, 1, 2, 10]);
}

#[rusty_tokio::test]
async fn stream_macro_can_be_empty_and_return_early() {
    let s = rusty_stream::stream! {
        if true {
            return;
        }
        yield 1u8;
    };
    let items: Vec<u8> = s.collect().await;
    assert!(items.is_empty());
}

fn numbers(fail_at: Option<u32>) -> BoxStream<'static, Result<u32, String>> {
    Box::pin(try_stream! {
        for n in 0..4u32 {
            if Some(n) == fail_at {
                Err(format!("failed at {n}"))?;
            }
            yield n;
        }
    })
}

#[rusty_tokio::test]
async fn try_stream_question_mark_ends_with_the_error() {
    let items: Vec<Result<u32, String>> = numbers(Some(2)).collect().await;
    assert_eq!(items, vec![Ok(0), Ok(1), Err("failed at 2".into())]);
    let all: Result<Vec<u32>, String> = numbers(None).try_collect().await;
    assert_eq!(all, Ok(vec![0, 1, 2, 3]));
}

#[rusty_tokio::test]
async fn try_stream_bare_return_and_nested_closures() {
    let s: BoxStream<'static, Result<i32, ()>> = Box::pin(try_stream! {
        // A bare `return;` inside a closure stays a plain return.
        let check = |n: i32| {
            if n > 1 {
                return;
            }
            assert!(n <= 1);
        };
        let mut inner = iter(vec![Ok::<i32, ()>(1), Ok(2), Ok(3)]);
        while let Some(n) = inner.next().await {
            let n = n?;
            check(n);
            yield n;
            if n == 2 {
                return;
            }
        }
    });
    let items: Vec<Result<i32, ()>> = s.collect().await;
    assert_eq!(items, vec![Ok(1), Ok(2)]);
}

#[test]
fn macro_streams_are_unpin_and_send() {
    fn assert_unpin_send<S: Stream + Unpin + Send>(_: &S) {}
    fn assert_try<S: Stream<Item = Result<u8, &'static str>>>(_: &S) {}
    let s = rusty_stream::stream! { yield 1u8; };
    assert_unpin_send(&s);
    let t = try_stream! { yield 1u8; Err::<(), &str>("e")?; };
    assert_try(&t);
    assert_unpin_send(&t);
}

#[cfg(feature = "futures-core")]
#[rusty_tokio::test]
async fn compat_round_trip() {
    use rusty_stream::Compat;
    // Inner Compat speaks futures_core::Stream; the outer one speaks ours.
    let theirs = Compat::new(iter(vec![1, 2, 3]));
    fn assert_futures_stream<S: futures_core::Stream>(_: &S) {}
    assert_futures_stream(&theirs);
    let ours: Vec<i32> = Compat::new(theirs).collect().await;
    assert_eq!(ours, vec![1, 2, 3]);
}

#[rusty_tokio::test]
async fn nested_stream_macro_owns_its_yields() {
    let outer = rusty_stream::stream! {
        let inner = rusty_stream::stream! {
            yield 1;
            yield 2;
        };
        let inner: Vec<i32> = inner.collect().await;
        yield inner.iter().sum::<i32>();
    };
    let items: Vec<i32> = outer.collect().await;
    assert_eq!(items, vec![3]);
}

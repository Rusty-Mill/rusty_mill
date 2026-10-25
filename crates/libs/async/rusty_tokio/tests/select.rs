use rusty_tokio::sync::oneshot;
use rusty_tokio::{select, Runtime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn two_branches_returns_the_first_ready_value() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            a = async { 1 } => a,
            b = std::future::pending::<i32>() => b,
        };
        assert_eq!(winner, 1);
    });
}

#[test]
fn three_branches_picks_whichever_actually_resolves() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            a = std::future::pending::<&str>() => a,
            b = async { "b" } => b,
            c = std::future::pending::<&str>() => c,
        };
        assert_eq!(winner, "b");
    });
}

#[test]
fn four_branches_picks_whichever_actually_resolves() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            a = std::future::pending::<&str>() => a,
            b = std::future::pending::<&str>() => b,
            c = async { "c" } => c,
            d = std::future::pending::<&str>() => d,
        };
        assert_eq!(winner, "c");
    });
}

#[test]
fn five_branches_picks_whichever_actually_resolves() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            a = std::future::pending::<&str>() => a,
            b = std::future::pending::<&str>() => b,
            c = std::future::pending::<&str>() => c,
            d = async { "d" } => d,
            e = std::future::pending::<&str>() => e,
        };
        assert_eq!(winner, "d");
    });
}

#[test]
fn earlier_branch_wins_when_both_are_immediately_ready() {
    // Under `biased;`, branches are polled in the order written on every
    // poll, so when two are simultaneously ready on the very first poll,
    // the earlier one wins. (Without `biased;` the start branch is random,
    // as in tokio -- see `default_mode_is_fair_between_always_ready_branches`.)
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            biased;
            a = async { "first" } => a,
            b = async { "second" } => b,
        };
        assert_eq!(winner, "first");
    });
}

#[test]
fn losing_branch_future_is_dropped_not_left_running() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let dropped = Arc::new(AtomicBool::new(false));
        struct MarkOnDrop(Arc<AtomicBool>);
        impl Drop for MarkOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let marker = MarkOnDrop(dropped.clone());
        let loser = async move {
            let _marker = marker;
            std::future::pending::<()>().await
        };

        select! {
            a = async {} => a,
            b = loser => b,
        };

        assert!(dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn select_waits_for_whichever_branch_wakes_later() {
    let rt = Runtime::builder().worker_threads(2).build().unwrap();
    rt.block_on(async {
        let (tx, rx) = oneshot::channel::<&str>();
        rusty_tokio::spawn(async move {
            rusty_tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = tx.send("late");
        });

        let winner = select! {
            slow = rx => slow.unwrap(),
            fast = std::future::pending::<&str>() => fast,
        };
        assert_eq!(winner, "late");
    });
}

#[test]
fn wildcard_pattern_discards_the_value() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner: i32 = select! {
            _ = async { 42 } => 7,
            _ = std::future::pending::<i32>() => 8,
        };
        assert_eq!(winner, 7);
    });
}

// ---------------------------------------------------------------------
// tokio syntax and semantics (#900, gap G6)
// ---------------------------------------------------------------------

use rusty_tokio::sync::mpsc;
use rusty_tokio::task::yield_now;
use std::cell::{Cell, RefCell};
use std::future::{pending, ready};

#[test]
fn refutable_pattern_mismatch_disables_the_branch_and_select_continues() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // `biased;` makes the mismatching branch complete first, every time.
        let winner = select! {
            biased;
            Some(x) = ready(None::<i32>) => x,
            v = ready(7) => v,
        };
        assert_eq!(winner, 7);

        // Unbiased: the mismatching branch is ready at once, the other
        // only on the second poll, so the select must keep going after
        // disabling the first branch.
        let winner = select! {
            Some(x) = ready(None::<i32>) => x,
            v = async {
                yield_now().await;
                8
            } => v,
        };
        assert_eq!(winner, 8);

        // The real shape: a closed channel's `None` disables the branch.
        let (tx, mut rx) = mpsc::channel::<i32>(1);
        drop(tx);
        let winner = select! {
            Some(x) = rx.recv() => x,
            v = async {
                yield_now().await;
                9
            } => v,
        };
        assert_eq!(winner, 9);
    });
}

#[test]
fn else_runs_when_every_branch_is_disabled() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let got = select! {
            Some(x) = ready(None::<i32>) => x,
            Ok(v) = ready(Err::<i32, ()>(())) => v,
            else => -1,
        };
        assert_eq!(got, -1);

        // Disabled by guards alone, before anything is polled.
        let got = select! {
            v = ready(1), if false => v,
            else => -2,
        };
        assert_eq!(got, -2);

        // `else` with no branches at all, as tokio allows.
        let got: i32 = select! { else => -3 };
        assert_eq!(got, -3);
    });
}

#[test]
fn false_guard_disables_the_branch_and_its_future_is_never_polled() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let order = RefCell::new(Vec::new());
        let polled = Cell::new(false);
        let winner = select! {
            v = {
                order.borrow_mut().push("future 0 built");
                async {
                    polled.set(true);
                    0
                }
            }, if { order.borrow_mut().push("guard 0"); false } => v,
            v = {
                order.borrow_mut().push("future 1 built");
                ready(1)
            }, if { order.borrow_mut().push("guard 1"); true } => v,
        };
        assert_eq!(winner, 1);
        assert!(
            !polled.get(),
            "a disabled branch's future must not be polled"
        );
        // tokio's order: every guard first, then every future expression --
        // disabled branches' included (built, never polled).
        assert_eq!(
            *order.borrow(),
            ["guard 0", "guard 1", "future 0 built", "future 1 built"]
        );
    });
}

#[test]
fn biased_polls_branches_in_declaration_order() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        for _ in 0..1000 {
            let winner = select! {
                biased;
                a = ready(1) => a,
                b = ready(2) => b,
            };
            assert_eq!(winner, 1);
        }
    });
}

#[test]
fn default_mode_is_fair_between_always_ready_branches() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut first, mut second) = (0u32, 0u32);
        for _ in 0..1000 {
            select! {
                _ = ready(()) => first += 1,
                _ = ready(()) => second += 1,
            }
        }
        // Each wins with probability 1/2 per round, so this fails only
        // with probability 2^-999.
        assert!(first > 0 && second > 0, "first={first} second={second}");
        assert_eq!(first + second, 1000);
    });
}

#[test]
fn eight_branches_pick_whichever_actually_resolves() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let winner = select! {
            a = pending::<u8>() => a,
            b = pending::<u8>() => b,
            c = pending::<u8>() => c,
            d = pending::<u8>() => d,
            e = pending::<u8>() => e,
            f = pending::<u8>() => f,
            g = ready(7u8) => g,
            h = pending::<u8>() => h,
        };
        assert_eq!(winner, 7);
    });
}

#[test]
#[should_panic(expected = "all branches are disabled and there is no else branch")]
fn panics_when_every_branch_is_disabled_and_there_is_no_else() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let _: i32 = select! {
            Some(x) = ready(None::<i32>) => x,
            v = ready(1), if false => v,
        };
    });
}

#[test]
fn handler_runs_in_the_callers_context_break_continue_return_and_question_mark() {
    async fn sum_until_error(items: Vec<Result<i32, String>>) -> Result<i32, String> {
        let mut items = items.into_iter();
        let mut total = 0;
        loop {
            let next = items.next();
            let n = select! {
                Some(item) = ready(next) => item?,
                else => break,
            };
            if n == 0 {
                continue;
            }
            total += n;
        }
        Ok(total)
    }

    async fn early_return(flag: bool) -> &'static str {
        if flag {
            select! {
                _ = ready(()) => return "returned",
            }
        }
        "fell through"
    }

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        assert_eq!(sum_until_error(vec![Ok(1), Ok(0), Ok(2)]).await, Ok(3));
        assert_eq!(
            sum_until_error(vec![Ok(1), Err("bad".into()), Ok(2)]).await,
            Err("bad".to_string())
        );
        assert_eq!(early_return(true).await, "returned");
        assert_eq!(early_return(false).await, "fell through");

        // `continue` and `break` with a value out of an enclosing loop,
        // from a block handler that also `.await`s.
        let mut rounds = 0;
        let got = loop {
            rounds += 1;
            select! {
                v = ready(rounds) => {
                    yield_now().await;
                    if v < 3 {
                        continue;
                    }
                    break v * 2;
                }
            }
        };
        assert_eq!(got, 6);
    });
}

#[test]
fn losing_futures_are_dropped_before_the_handler_runs() {
    struct MarkOnDrop<'a>(&'a Cell<bool>);
    impl Drop for MarkOnDrop<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let dropped = Cell::new(false);
        let marker = MarkOnDrop(&dropped);
        let loser = async move {
            let _marker = marker;
            pending::<()>().await
        };
        let seen_in_handler = select! {
            biased;
            _ = ready(()) => dropped.get(),
            _ = loser => false,
        };
        assert!(
            seen_in_handler,
            "the losing future was still alive in the handler"
        );

        // ...which is also what lets a handler mutably borrow what a
        // future borrowed.
        let mut log = Vec::new();
        select! {
            () = async { log.push(1) } => log.push(2),
        }
        assert_eq!(log, [1, 2]);
    });
}

// ---------------------------------------------------------------------
// The syntax forms the workspace's `tokio::select!` sites use, each
// written out so a migration to `rusty_tokio::select!` is a rename.
// ---------------------------------------------------------------------

/// agentgateway `serve.rs:103`, adk-a2a `executor.rs:248`, rusty_lsp
/// `server.rs:436`: expression handlers that `break`/`continue`, a
/// `match` expression handler, and block handlers that `break <value>`
/// or `return`.
#[test]
fn workspace_form_loop_control_handlers() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut attempts = 0;
        let accepted: Option<i32> = loop {
            attempts += 1;
            let stream = select! {
                _ = pending::<()>() => break None,
                accepted = ready(if attempts < 3 { Err("transient") } else { Ok(attempts) }) => match accepted {
                    Ok(stream) => stream,
                    Err(_err) => {
                        continue;
                    }
                },
            };
            break Some(stream);
        };
        assert_eq!(accepted, Some(3));

        let mut tries = 0;
        let read: Result<i32, &str> = loop {
            tries += 1;
            let read = select! {
                result = pending::<i32>() => result,
                _ = ready(()), if tries >= 2 => {
                    break Err("writer failed");
                }
                _ = pending::<()>() => break Ok(0),
                _ = ready(()), if tries < 2 => continue,
            };
            break Ok(read);
        };
        assert_eq!(read, Err("writer failed"));

        async fn adk_like() -> Result<(), ()> {
            let next = select! {
                biased;
                _ = ready(()) => {
                    return Ok(());
                }
                next = pending::<Option<i32>>() => next,
            };
            let Some(_event) = next else { return Err(()) };
            Err(())
        }
        assert_eq!(adk_like().await, Ok(()));
    });
}

/// agentgateway `serve.rs:188`/`:215`, ts-daemon, rusty_mill_board,
/// rk-app `mcp_server.rs:91`, rk-app `acp.rs:85`: block handlers with no
/// comma between arms, empty `{}` handlers, `_` and `()` patterns, a
/// pinned future polled through `.as_mut()`, and `.await` in a handler.
#[test]
fn workspace_form_block_handlers_without_commas() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut connection = std::pin::pin!(async { Err::<(), &str>("closed") });
        let mut closed_with_error = false;
        select! {
            result = connection.as_mut() => {
                if let Err(_err) = result {
                    closed_with_error = true;
                }
            }
            _ = pending::<()>() => {
                yield_now().await;
            }
        }
        assert!(closed_with_error);

        let ctrl_c = async {};
        let terminate = pending::<()>();
        select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
    });
}

/// rk-app `mcp_server.rs:114`, rusty_acp `routes.rs:514`/`:524`, and
/// rusty_tonic `streaming.rs:321` (a bare imported `select!` as the tail
/// of an `async` block): `select!` as a value -- a function's tail with
/// `?` chains in an expression handler, and an arm of an outer `match`
/// with a `return` from a block handler.
#[test]
fn workspace_form_select_as_a_value() {
    async fn serve(done: Result<Result<(), String>, String>) -> Result<(), String> {
        select! {
            done = ready(done) => done
                .map_err(|e| format!("thread ended: {e}"))?
                .map_err(|e| format!("server: {e}")),
            () = pending::<()>() => Ok(()),
        }
    }

    async fn sync_request(deadline: Option<()>) -> Result<Option<i32>, ()> {
        let next = pending::<Option<i32>>();
        let tick = ready(());
        let notification = match deadline {
            Some(_deadline) => select! {
                notification = next => notification,
                _ = pending::<()>() => None,
                _ = ready(()) => {
                    return Ok(None);
                }
            },
            None => select! {
                notification = next => notification,
                _ = tick => Some(1),
            },
        };
        Ok(notification)
    }

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        assert_eq!(serve(Ok(Ok(()))).await, Ok(()));
        assert_eq!(serve(Err("x".into())).await, Err("thread ended: x".into()));
        assert_eq!(sync_request(Some(())).await, Ok(None));
        assert_eq!(sync_request(None).await, Ok(Some(1)));

        let outcome = async {
            select! {
                r = ready(Ok::<u8, ()>(1)) => Some(r),
                _ = pending::<()>() => None,
            }
        }
        .await;
        assert_eq!(outcome, Some(Ok(1)));
    });
}

/// rk-agui `lib.rs:212`, rusty_acp `agent.rs:366`/`mod.rs:1489`:
/// `biased;` with refutable `Some(..)` patterns, `?` inside expression
/// and block handlers, `&mut handle` of a `JoinHandle` as the future,
/// `break <value>?` out of the loop, and `(&mut handle).await` in a
/// handler after the select has dropped its borrow.
#[test]
fn workspace_form_biased_refutable_and_join_handles() {
    async fn relay() -> Result<i32, String> {
        let (tx, mut approvals) = mpsc::channel::<i32>(1);
        let (tok_tx, mut tokens) = mpsc::channel::<i32>(4);
        drop(tx); // approvals closed: its `Some(..)` branch disables itself
        let mut turn = rusty_tokio::spawn(async move {
            for t in [1, 2] {
                let _ = tok_tx.send(t).await;
            }
            Ok::<i32, String>(40)
        });
        let mut chunks = 0;
        loop {
            select! {
                biased;
                Some(request) = approvals.recv() => {
                    return Err(format!("unexpected approval {request}"));
                }
                Some(delta) = tokens.recv() => chunks += Ok::<i32, String>(delta)?,
                joined = &mut turn => {
                    while let Some(delta) = tokens.recv().await {
                        chunks += delta;
                    }
                    break joined.map_err(|e| e.to_string())?.map(|v| v + chunks);
                }
            }
        }
    }

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        assert_eq!(relay().await, Ok(43));

        // A task that ends on its own shortly: awaiting an aborted task
        // that would otherwise pend forever is a separate runtime
        // question this test shouldn't depend on.
        let mut agent_task =
            rusty_tokio::spawn(rusty_tokio::time::sleep(Duration::from_millis(50)));
        let outcome = select! {
            biased;
            _ = ready(()) => {
                agent_task.abort();
                let _ = (&mut agent_task).await;
                "cancelled"
            }
            _ = &mut agent_task => "finished",
        };
        assert_eq!(outcome, "cancelled");
    });
}

/// ts-engine `lib.rs:467`, ts-net `stack.rs:167`: many branches mixing
/// refutable patterns, `, if` guards over futures that must still build
/// when the guard is false, `match` expression handlers, a handler that
/// mutably borrows `self` while the futures borrowed its fields, and
/// `else => break`.
#[test]
fn workspace_form_guards_else_and_self_borrows() {
    struct Engine {
        netmap_rx: mpsc::Receiver<i32>,
        cmd_rx: mpsc::Receiver<i32>,
        stack_rx: Option<mpsc::Receiver<i32>>,
        seen: Vec<i32>,
    }

    async fn recv_opt(rx: &mut Option<mpsc::Receiver<i32>>) -> Option<i32> {
        match rx {
            Some(rx) => rx.recv().await,
            None => pending().await,
        }
    }

    async fn recv_tun(tun: &Option<i32>) -> Option<Result<i32, String>> {
        match tun {
            Some(v) => Some(Ok(*v)),
            None => pending().await,
        }
    }

    impl Engine {
        async fn apply(&mut self, v: i32) {
            yield_now().await;
            self.seen.push(v);
        }

        async fn run(&mut self) {
            let tun: Option<i32> = None;
            let delay: Option<u64> = None;
            loop {
                select! {
                    Some(resp) = self.netmap_rx.recv() => self.apply(resp).await,
                    Some(cmd) = self.cmd_rx.recv() => self.apply(cmd * 10).await,
                    res = recv_tun(&tun), if tun.is_some() => match res {
                        Some(Ok(pkt)) => self.apply(pkt).await,
                        Some(Err(_e)) => {}
                        None => {}
                    },
                    _ = pending::<()>(), if delay.is_some() => {}
                    Some(pkt) = recv_opt(&mut self.stack_rx) => self.apply(pkt * 100).await,
                    else => break,
                }
            }
        }
    }

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (netmap_tx, netmap_rx) = mpsc::channel(4);
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        let (stack_tx, stack_rx) = mpsc::channel(4);
        netmap_tx.send(1).await.unwrap();
        cmd_tx.send(2).await.unwrap();
        stack_tx.send(3).await.unwrap();
        drop((netmap_tx, cmd_tx, stack_tx));

        let mut engine = Engine {
            netmap_rx,
            cmd_rx,
            stack_rx: Some(stack_rx),
            seen: Vec::new(),
        };
        engine.run().await;
        engine.seen.sort_unstable();
        assert_eq!(engine.seen, [1, 20, 300]);
    });
}

/// rusty_db `pool_stats*.rs`: `_ = &mut boxed_future` with a diverging
/// `panic!` handler, raced against a timer.
#[test]
fn workspace_form_mut_ref_future_and_diverging_handler() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut blocked = Box::pin(pending::<()>());
        select! {
            _ = &mut blocked => panic!("should still be blocked"),
            _ = rusty_tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    });
}

/// rusty-meshed-domains `run_continuous.rs:330`: a tuple pattern over
/// an inline `async` block that borrows locals mutably, with a block
/// handler using those bindings.
#[test]
fn workspace_form_tuple_pattern_over_async_block() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut rng = 4u32;
        let mut total = 0;
        let interrupted = select! {
            (id, events) = async {
                rng += 1;
                yield_now().await;
                (format!("id-{rng}"), 3)
            } => {
                total += events;
                assert_eq!(&id[..id.len().min(4)], "id-5");
                false
            },
            _ = pending::<()>() => true,
        };
        assert!(!interrupted);
        assert_eq!(total, 3);
    });
}

/// `mut`/`ref` bindings in a pattern are accepted (the pre-match check
/// strips them; the handler's own match keeps them).
#[test]
fn mut_and_ref_bindings_are_accepted() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let got = select! {
            Some(mut v) = ready(Some(vec![1])) => {
                v.push(2);
                v
            }
        };
        assert_eq!(got, [1, 2]);
        let got = select! {
            (ref a, b) = ready((String::from("x"), 1)) => format!("{a}{b}"),
        };
        assert_eq!(got, "x1");
    });
}

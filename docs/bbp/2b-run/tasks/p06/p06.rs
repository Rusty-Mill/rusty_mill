use target::Bucket;
#[test] fn starts_full_and_is_all_or_nothing() { let mut b = Bucket::new(10, 10); assert!(!b.try_take(11, 0)); assert!(b.try_take(10, 0)); assert!(!b.try_take(1, 0)); }
#[test] fn refills_over_time() { let mut b = Bucket::new(10, 10); assert!(b.try_take(10, 0)); assert!(b.try_take(1, 100)); assert!(!b.try_take(1, 100)); }
#[test] fn keeps_the_fraction() { let mut b = Bucket::new(3, 3); assert!(b.try_take(3, 0)); assert!(!b.try_take(1, 333)); assert!(b.try_take(1, 334)); }
#[test] fn refusal_costs_nothing() { let mut b = Bucket::new(5, 1); assert!(b.try_take(3, 0)); assert!(!b.try_take(3, 0)); assert!(b.try_take(2, 0)); }
#[test] fn caps_at_capacity() { let mut b = Bucket::new(2, 100); assert!(b.try_take(2, 0)); assert!(!b.try_take(3, 60_000)); assert!(b.try_take(2, 60_000)); }
#[test] fn zero_and_backwards_clock() { let mut b = Bucket::new(1, 1); assert!(b.try_take(0, 0)); assert!(b.try_take(1, 5_000)); assert!(!b.try_take(1, 1_000)); assert!(b.try_take(0, 1_000)); }

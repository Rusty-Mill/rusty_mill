use target::Policy;
fn p(deadline_ms: u64) -> Policy { Policy { base_ms: 100, factor: 2, max_delay_ms: 1_000, max_attempts: 6, deadline_ms } }
#[test] fn grows_and_caps() { let q = p(0); assert_eq!(q.next_delay(1, 0), Some(100)); assert_eq!(q.next_delay(3, 0), Some(400)); assert_eq!(q.next_delay(5, 0), Some(1_000)); }
#[test] fn stops_at_max_attempts() { assert_eq!(p(0).next_delay(6, 0), None); }
#[test] fn respects_deadline() { let q = p(1_000); assert_eq!(q.next_delay(2, 800), Some(200)); assert_eq!(q.next_delay(2, 801), None); }
#[test] fn saturates() { let q = Policy { base_ms: u64::MAX / 2, factor: 10, max_delay_ms: u64::MAX, max_attempts: 100, deadline_ms: 0 }; assert_eq!(q.next_delay(50, 0), Some(u64::MAX)); }
#[test] fn deadline_with_huge_elapsed() { assert_eq!(p(1_000).next_delay(1, u64::MAX), None); }
#[test] fn zero_failures() { assert_eq!(p(0).next_delay(0, 0), None); }

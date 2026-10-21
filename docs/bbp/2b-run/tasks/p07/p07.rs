use target::Cache;
#[test] fn evicts_least_recent() { let mut c = Cache::new(2, 1_000); c.insert("a", "1", 0); c.insert("b", "2", 0); assert_eq!(c.get("a", 1).as_deref(), Some("1")); c.insert("c", "3", 2); assert_eq!(c.get("b", 3), None); assert_eq!(c.get("a", 3).as_deref(), Some("1")); }
#[test] fn expires() { let mut c = Cache::new(4, 100); c.insert("a", "1", 0); assert_eq!(c.get("a", 99).as_deref(), Some("1")); assert_eq!(c.get("a", 100), None); assert_eq!(c.len(100), 0); }
#[test] fn get_does_not_extend() { let mut c = Cache::new(4, 100); c.insert("a", "1", 0); c.get("a", 90); assert_eq!(c.get("a", 150), None); }
#[test] fn reinsert_restarts_life() { let mut c = Cache::new(4, 100); c.insert("a", "1", 0); c.insert("a", "2", 90); assert_eq!(c.get("a", 150).as_deref(), Some("2")); }
#[test] fn expired_go_before_lru() { let mut c = Cache::new(2, 100); c.insert("old", "1", 0); c.insert("new", "2", 50); assert!(c.get("old", 60).is_some()); c.insert("x", "3", 120); assert_eq!(c.get("new", 121).as_deref(), Some("2")); assert_eq!(c.get("x", 121).as_deref(), Some("3")); }
#[test] fn zero_capacity() { let mut c = Cache::new(0, 100); c.insert("a", "1", 0); assert_eq!(c.get("a", 0), None); assert_eq!(c.len(0), 0); }

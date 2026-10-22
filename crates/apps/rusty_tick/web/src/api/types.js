/**
 * The wire model, exactly as `rusty_tick`'s API speaks it: camelCase, ids are
 * UUIDs, times are Unix milliseconds, and `etag` is the record's version.
 *
 * (The reference prompt sketched TickTick's own model — 24-hex ids, ISO dates,
 * `status: 0|2`. The backend here is `rusty_tick` on `rusty_multimodal_db`, so
 * the UI follows its contract instead; see README "Where this differs".)
 */
export {};

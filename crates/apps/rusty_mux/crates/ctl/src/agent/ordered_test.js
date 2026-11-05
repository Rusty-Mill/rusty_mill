// Run by `ordered_reports_keep_their_order` after `ordered.js`: a fake exec
// that holds each send until the test ends it.
const assert = (cond, what) => {
  if (!cond) throw new Error(what);
};
const sent = [];
const pending = [];
const send = ordered((args, done) => {
  sent.push(args[0]);
  pending.push(done);
});

// A slow first send: later reports wait, and only the newest is kept.
send(["working"]);
send(["blocked"]);
send(["idle"]);
assert(sent.join() === "working", `while the first runs: ${sent}`);
pending.shift()();
assert(sent.join() === "working,idle", `after it ends: ${sent}`);
pending.shift()();

// A stuck send holds one waiting report, however many arrive, and each
// call returns at once.
send(["stuck"]);
const start = Date.now();
for (let i = 0; i < 10000; i++) send([`late${i}`]);
assert(Date.now() - start < 1000, "calls returned at once");
assert(sent.length === 3 && pending.length === 1, `one send in flight: ${sent}`);
pending.shift()();
assert(sent[sent.length - 1] === "late9999", `the newest goes next: ${sent.slice(-1)}`);
pending.shift()();

// A send that throws, or ends twice, frees the sender once.
let calls = 0;
const throwing = ordered((args, done) => {
  calls += 1;
  if (args[0] === "boom") throw new Error("exec failed");
  done();
  done();
});
throwing(["boom"]);
throwing(["after"]);
throwing(["again"]);
assert(calls === 3, `each report sent once: ${calls}`);
console.log("ok");

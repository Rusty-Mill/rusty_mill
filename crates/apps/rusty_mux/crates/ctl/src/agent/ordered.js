// Sends one status report at a time. While one runs, only the newest
// waiting report is kept, so an older report never lands after a newer
// one and nothing piles up behind a slow or stuck send.
const ordered = (exec) => {
  let busy = false;
  let next = null;
  const run = (args) => {
    busy = true;
    let ended = false;
    const finish = () => {
      if (ended) return;
      ended = true;
      busy = false;
      const waiting = next;
      next = null;
      if (waiting) run(waiting);
    };
    try {
      exec(args, finish);
    } catch {
      finish();
    }
  };
  return (args) => {
    if (busy) next = args;
    else run(args);
  };
};

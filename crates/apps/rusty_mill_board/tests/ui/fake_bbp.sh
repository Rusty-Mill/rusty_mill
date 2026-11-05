#!/bin/sh
# A stand-in `bbp` for tests/ui/decisions.mjs and task_paused.mjs: one stuck question with two
# candidates (or, while $FAKE_BBP_DIR/paused exists, one #598 task pause), until any
# committed action, after which the inbox is empty. While $FAKE_BBP_DIR/fail exists every call
# fails, as a host whose log is unreadable does. Logs every call to $FAKE_BBP_DIR/calls.
D=${FAKE_BBP_DIR:?set FAKE_BBP_DIR}
echo "$@" >> $D/calls
if [ -f $D/fail ]; then echo "log unreadable" >&2; exit 1; fi
case "$1" in inbox)
  if [ -f $D/done ]; then echo '[]'; exit 0; fi
  if [ -f $D/paused ]; then
    echo '[{"id":"T1/task/9","task":"T1","brief":2,"rev":7,"seq":9,"reason":"TaskPaused","claim":null,"stakes":null,"reversible":null,"evidence":[],"candidates":[],"mark":null,"candidate":null,"acted":[],"totals":[]}]'
    exit 0
  fi
  cat <<'J'
[{"id":"T1/msg:1/14","task":"T1","rev":16,"seq":14,"reason":"Stuck","claim":{"id":1,"text":"Which cache?","status":"Escalated","score":0,"confidence":50,"epoch":0,"spent":4,"budget":8,"evidence":[]},"stakes":"Medium","reversible":true,"evidence":[{"msg":2,"author":"architect","direction":"Support","kind":"Cited","delta":2,"epoch":0}],"candidates":[{"id":5,"text":"Use an LRU in memory","status":"Open","score":4,"confidence":70,"epoch":0,"spent":2,"budget":8},{"id":6,"text":"Use redis","status":"Open","score":4,"confidence":70,"epoch":0,"spent":2,"budget":8}],"mark":null,"candidate":null,"brief":2,"acted":[],"totals":[]}]
J
  exit 0;; esac
case "$*" in *"--dry-run yes"*) echo '{"rev":16,"response":"Ok","events":[{"ClaimDecided":{}},{"ClaimAccepted":{}},{"ClaimDropped":{}}]}'; exit 0;; esac
touch $D/done; echo '"Ok"'

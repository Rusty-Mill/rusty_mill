#!/bin/sh
# A stand-in `bbp` for tests/ui/decisions.mjs: one stuck question with two candidates, until any
# committed action, after which the inbox is empty. Logs every call to $FAKE_BBP_DIR/calls.
D=${FAKE_BBP_DIR:?set FAKE_BBP_DIR}
echo "$@" >> $D/calls
case "$1" in inbox)
  if [ -f $D/done ]; then echo '[]'; exit 0; fi
  cat <<'J'
[{"id":"T1/msg:1/14","task":"T1","rev":16,"seq":14,"reason":"Stuck","claim":{"id":1,"text":"Which cache?","status":"Escalated","score":0,"confidence":50,"epoch":0,"spent":4,"budget":8,"evidence":[]},"stakes":"Medium","reversible":true,"evidence":[{"msg":2,"author":"architect","direction":"Support","kind":"Cited","delta":2,"epoch":0}],"candidates":[{"id":5,"text":"Use an LRU in memory","status":"Open","score":4,"confidence":70,"epoch":0,"spent":2,"budget":8},{"id":6,"text":"Use redis","status":"Open","score":4,"confidence":70,"epoch":0,"spent":2,"budget":8}],"mark":null,"candidate":null}]
J
  exit 0;; esac
case "$*" in *"--dry-run yes"*) echo '{"rev":16,"response":"Ok","events":[{"ClaimDecided":{}},{"ClaimAccepted":{}},{"ClaimDropped":{}}]}'; exit 0;; esac
touch $D/done; echo '"Ok"'

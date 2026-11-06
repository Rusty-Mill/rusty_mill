#!/bin/sh
# A stand-in `bbp` for tests/ui/decisions.mjs, task_paused.mjs and erosion.mjs: one stuck question
# with two candidates (or, while $FAKE_BBP_DIR/paused exists, one #598 task pause; while
# $FAKE_BBP_DIR/erosion exists, a claim erosion fails, one
# whose detail has an unexpected shape, and one it passes report-only), until any
# committed action, after which the inbox is empty. While $FAKE_BBP_DIR/fail exists every call
# fails, as a host whose log is unreadable does. Logs every call to $FAKE_BBP_DIR/calls.
D=${FAKE_BBP_DIR:?set FAKE_BBP_DIR}
echo "$@" >> $D/calls
if [ -f $D/fail ]; then echo "log unreadable" >&2; exit 1; fi
case "$1" in inbox)
  if [ -f $D/done ]; then echo '[]'; exit 0; fi
  if [ -f $D/erosion ]; then
    C='"status":"Escalated","score":9,"confidence":90,"epoch":0,"spent":2,"budget":8,"evidence":[]'
    S='"stakes":"Low","reversible":false,"evidence":[],"candidates":[],"mark":null,"candidate":null,"brief":2,"acted":[],"totals":[]'
    L='"concentration_bp":{"base":0,"candidate":0,"delta":0},"duplication_bp":{"base":0,"candidate":4800,"delta":4800},"total_cc":2,"loc":11,"functions":1,"worst_affected":[{"key":"src/b.rs::grand","base_cc":0,"cc":2}],"unmeasured":["tool.py"],"parse_errors":[]'
    echo "[{\"id\":\"T1/msg:1/14\",\"task\":\"T1\",\"rev\":16,\"seq\":14,\"reason\":\"Irreversible\",\"claim\":{\"id\":1,\"text\":\"Paste the helper\",$C},$S,\"erosion\":{\"report\":9,\"failing\":true,\"error\":null,\"line\":{\"erosion\":1,\"base\":\"a\",\"mode\":{\"max_concentration_bp\":50,\"max_duplication_bp\":50},\"within_limits\":false,$L}}},\
{\"id\":\"T1/msg:4/16\",\"task\":\"T1\",\"rev\":16,\"seq\":16,\"reason\":\"Irreversible\",\"claim\":{\"id\":4,\"text\":\"Odd detail\",$C},$S,\"erosion\":{\"report\":11,\"failing\":true,\"line\":{\"mode\":null,\"worst_affected\":[null],\"unmeasured\":\"x\"}}},\
{\"id\":\"T1/msg:3/15\",\"task\":\"T1\",\"rev\":16,\"seq\":15,\"reason\":\"Irreversible\",\"claim\":{\"id\":3,\"text\":\"Ship the script\",$C},$S,\"erosion\":{\"report\":10,\"failing\":false,\"error\":null,\"line\":{\"erosion\":1,\"base\":\"a\",\"mode\":\"report-only\",\"within_limits\":null,$L}}}]"
    exit 0
  fi
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

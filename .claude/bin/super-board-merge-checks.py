#!/usr/bin/env python3
"""Pure merge-gate decisions, kept out of bash so they can be unit tested.

  method  --config C --settings S   which `gh pr merge` flag this repo allows
  target  --expect BASE --meta M    the PR still targets the configured base
  ci      --head SHA --meta M       current-head checks are present and green

Exit codes (the gate passes them through):
  0   ok
  9   target mismatch, or no allowed merge method (nothing merges)
  10  CI still pending for the current head (try again later)
  11  CI failed for the current head
  12  CI evidence missing, unreadable, or for a different head (fail closed)
"""
from __future__ import annotations

import argparse
import json
import re
import sys

FLAGS = {'merge': '--merge', 'squash': '--squash', 'rebase': '--rebase'}
SETTING = {'merge': 'mergeCommitAllowed', 'squash': 'squashMergeAllowed', 'rebase': 'rebaseMergeAllowed'}
OK_CONCLUSIONS = {'SUCCESS', 'NEUTRAL', 'SKIPPED'}
BAD_CONCLUSIONS = {'FAILURE', 'TIMED_OUT', 'CANCELLED', 'ACTION_REQUIRED', 'STARTUP_FAILURE', 'STALE'}


class Refuse(Exception):
    def __init__(self, code, why):
        super().__init__(why)
        self.code = code


def choose_method(config, settings):
    """Config `merge_policy.method` (default "merge": CONTRIBUTING.md forbids squash).

    Fails closed when the repo does not allow it: never silently fall back to another
    method, and never touch repository settings.
    """
    want = (config.get('merge_policy') or {}).get('method', 'merge')
    if want not in FLAGS:
        raise Refuse(9, f'merge_policy.method must be one of {sorted(FLAGS)}, got {want!r}')
    if settings.get(SETTING[want]) is not True:
        allowed = [m for m in FLAGS if settings.get(SETTING[m]) is True]
        raise Refuse(9, f'repository does not allow {want} merges (allowed: {allowed or "none"}); '
                        'set merge_policy.method to an allowed one, do not change repository settings')
    return want


def check_target(expect_base, meta):
    actual = meta.get('baseRefName')
    if not isinstance(actual, str) or not actual:
        raise Refuse(12, 'PR target branch unreadable')
    if actual != expect_base:
        raise Refuse(9, f'PR targets {actual!r} but the board is configured for {expect_base!r}; '
                        'refusing to merge a retargeted or off-base PR')


def evaluate_ci(meta, head, required=None):
    """Return normally when every check on `head` passed; otherwise raise Refuse."""
    if meta.get('headRefOid') != head:
        raise Refuse(12, f'CI was read for head {meta.get("headRefOid")}, not the head under merge ({head})')
    rollup = meta.get('statusCheckRollup')
    if not isinstance(rollup, list) or not rollup:
        raise Refuse(12, 'no CI checks reported for this head; cannot prove it is green')
    pending, failed, seen = [], [], set()
    for entry in rollup:
        if not isinstance(entry, dict):
            raise Refuse(12, 'CI check entry unreadable')
        name = entry.get('name') or entry.get('context')
        if not isinstance(name, str) or not name:
            raise Refuse(12, 'CI check without a name')
        if 'state' in entry and 'conclusion' not in entry:  # commit status context
            state = entry.get('state')
            verdict = 'ok' if state == 'SUCCESS' else 'bad' if state in ('FAILURE', 'ERROR') else 'pending'
        elif entry.get('status') != 'COMPLETED':
            verdict = 'pending'
        else:
            conclusion = entry.get('conclusion')
            verdict = ('ok' if conclusion in OK_CONCLUSIONS else
                       'bad' if conclusion in BAD_CONCLUSIONS or not conclusion else 'pending')
        seen.add(name)
        if verdict == 'bad':
            failed.append(name)
        elif verdict == 'pending':
            pending.append(name)
    for name in required or []:
        if name not in seen:
            raise Refuse(12, f'required check {name!r} has not reported for this head')
    if failed:
        raise Refuse(11, 'failing checks on the current head: ' + ', '.join(sorted(set(failed))))
    if pending:
        raise Refuse(10, 'checks still running on the current head: ' + ', '.join(sorted(set(pending))))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='cmd', required=True)
    m = sub.add_parser('method')
    m.add_argument('--config', required=True)
    m.add_argument('--settings', required=True)
    t = sub.add_parser('target')
    t.add_argument('--expect', required=True)
    t.add_argument('--meta', required=True)
    c = sub.add_parser('ci')
    c.add_argument('--head', required=True)
    c.add_argument('--meta', required=True)
    c.add_argument('--config')
    a = p.parse_args(argv)

    def load(path):
        with open(path, encoding='utf-8') as stream:
            return json.load(stream)

    try:
        if a.cmd == 'method':
            print(FLAGS[choose_method(load(a.config), load(a.settings))])
        elif a.cmd == 'target':
            check_target(a.expect, load(a.meta))
        else:
            if not re.fullmatch(r'[0-9a-f]{40}', a.head):
                raise Refuse(12, 'head is not a full commit id')
            cfg = load(a.config) if a.config else {}
            if cfg.get('ci_required') is False:
                return 0
            evaluate_ci(load(a.meta), a.head, cfg.get('ci_required_checks'))
        return 0
    except Refuse as e:
        print(f'[merge-checks] {e}', file=sys.stderr)
        return e.code
    except (OSError, ValueError, TypeError, AttributeError) as e:
        print(f'[merge-checks] unreadable input: {e}', file=sys.stderr)
        return 12


if __name__ == '__main__':
    sys.exit(main())

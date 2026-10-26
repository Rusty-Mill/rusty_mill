#!/usr/bin/env python3
"""Stop / PreCompact hook: capture a cloud session's transcript in remind-me (ADR-0029).

A claude.ai Projects session runs in a fresh container with no rusty-remind-me
binary, no user-level hooks and a transcript that dies with the container. This
sends the transcript to the remote remind-me server's MCP endpoint
(`remind_me_capture_transcript`), which keeps one capture per session, replaced
in place on every call.

Runs only when all three are set, so a local CLI session stays with the plugin hook:
  CLAUDE_CODE_REMOTE=true   set by Claude Code in a cloud session
  REMIND_ME_URL             the server's MCP endpoint, e.g. https://memory.example/mcp
  REMIND_ME_TOKEN           its bearer token (an environment secret, never committed)

Like the plugin's capture.sh: Stop is skipped when re-entrant and throttled to once a
minute; PreCompact always runs. SessionEnd is not used, because a reclaimed container
may never fire it. Every failure is reported on stderr and the hook exits 0.
"""
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

# remind_me_core::transcript::MAX_TRANSCRIPT_BYTES: fits the server's 4 MiB body.
MAX_TRANSCRIPT_BYTES = 3 * 1024 * 1024
THROTTLE_SECONDS = 60
TIMEOUT_SECONDS = 20
PROTOCOL_VERSION = '2025-06-18'


def enabled(env):
    return (env.get('CLAUDE_CODE_REMOTE') == 'true'
            and bool(env.get('REMIND_ME_URL')) and bool(env.get('REMIND_ME_TOKEN')))


def tail(raw, limit=MAX_TRANSCRIPT_BYTES):
    """The newest whole lines of `raw` that fit in `limit` bytes."""
    if len(raw) <= limit:
        return raw
    cut = raw[-limit:]
    newline = cut.find(b'\n')
    return cut[newline + 1:] if newline >= 0 else b''


def throttled(session_id, now, state_dir):
    """True when this session was captured on Stop less than a minute ago; else stamp it."""
    stamp = pathlib.Path(state_dir) / (re.sub(r'[^A-Za-z0-9_-]', '_', session_id) + '.last')
    try:
        if now - float(stamp.read_text()) < THROTTLE_SECONDS:
            return True
    except (OSError, ValueError):
        pass
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(str(now))
    return False


def git(cwd, *args):
    try:
        r = subprocess.run(['git', '-C', cwd, *args], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if r.returncode:
        return None
    return r.stdout.strip() or None


def without_userinfo(url):
    """A remote URL with any credentials removed: a proxy remote can carry a token."""
    if not url:
        return None
    parts = urllib.parse.urlsplit(url)
    if not parts.netloc or '@' not in parts.netloc:
        return url
    host = parts.netloc.rsplit('@', 1)[1]
    return urllib.parse.urlunsplit(parts._replace(netloc=host))


def context(cwd):
    top = git(cwd, 'rev-parse', '--show-toplevel')
    fields = {
        'project': os.path.basename(top or cwd) or None,
        'git_branch': git(cwd, 'rev-parse', '--abbrev-ref', 'HEAD'),
        'git_sha': git(cwd, 'rev-parse', 'HEAD'),
        'git_remote': without_userinfo(git(cwd, 'remote', 'get-url', 'origin')),
        'cwd': cwd,
    }
    return {k: v for k, v in fields.items() if v}


class Mcp:
    """Just enough Streamable HTTP MCP client to make one tool call."""

    def __init__(self, url, token):
        self.url, self.token, self.session, self.next_id = url, token, None, 1

    def post(self, message):
        headers = {'Content-Type': 'application/json',
                   'Accept': 'application/json, text/event-stream',
                   'Authorization': f'Bearer {self.token}',
                   'MCP-Protocol-Version': PROTOCOL_VERSION}
        if self.session:
            headers['Mcp-Session-Id'] = self.session
        request = urllib.request.Request(self.url, json.dumps(message).encode(), headers)
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            self.session = response.headers.get('Mcp-Session-Id') or self.session
            body = response.read().decode('utf-8', 'replace')
            kind = response.headers.get('Content-Type', '')
        if 'id' not in message:
            return None
        return reply_to(message['id'], body, 'text/event-stream' in kind)

    def request(self, method, params):
        message = {'jsonrpc': '2.0', 'id': self.next_id, 'method': method, 'params': params}
        self.next_id += 1
        reply = self.post(message)
        if reply is None or 'error' in reply:
            raise RuntimeError(f'{method}: {reply and reply["error"]}')
        return reply['result']

    def call(self, tool, arguments):
        self.request('initialize', {'protocolVersion': PROTOCOL_VERSION, 'capabilities': {},
                                    'clientInfo': {'name': 'remind-me-capture-hook', 'version': '1'}})
        self.post({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        result = self.request('tools/call', {'name': tool, 'arguments': arguments})
        if result.get('isError'):
            raise RuntimeError(''.join(c.get('text', '') for c in result.get('content', [])))
        return result


def reply_to(message_id, body, sse):
    """The JSON-RPC reply to `message_id` in a plain or event-stream body."""
    chunks = [line[5:].strip() for line in body.splitlines() if line.startswith('data:')] if sse else [body]
    for chunk in chunks:
        try:
            reply = json.loads(chunk)
        except ValueError:
            continue
        if isinstance(reply, dict) and reply.get('id') == message_id:
            return reply
    return None


def run(payload, env, now, state_dir):
    """Capture the session; returns what happened, for the log line and tests."""
    if not enabled(env):
        return 'off'
    event = payload.get('hook_event_name', '')
    session_id = payload.get('session_id') or ''
    path = payload.get('transcript_path') or ''
    if not session_id or not os.path.isfile(path):
        return 'nothing to capture'
    if event == 'Stop':
        if payload.get('stop_hook_active'):
            return 're-entrant'
        if throttled(session_id, now, state_dir):
            return 'throttled'
    transcript = tail(pathlib.Path(path).read_bytes()).decode('utf-8', 'replace')
    if not transcript.strip():
        return 'nothing to capture'
    arguments = {'session_id': session_id, 'transcript': transcript,
                 'reason': 'precompact' if event == 'PreCompact' else 'stop',
                 **context(payload.get('cwd') or os.getcwd())}
    Mcp(env['REMIND_ME_URL'], env['REMIND_ME_TOKEN']).call('remind_me_capture_transcript', arguments)
    return 'captured'


def main():
    try:
        payload = json.load(sys.stdin)
        state_dir = os.path.join(tempfile.gettempdir(), 'remind-me-capture')
        run(payload, os.environ, time.time(), state_dir)
    except Exception as e:  # a hook must never fail the session
        print(f'remind-me-capture: {type(e).__name__}: {e}', file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main())

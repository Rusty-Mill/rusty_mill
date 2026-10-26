"""remind-me-capture.py: the cloud session capture hook (ADR-0029).

Run: python3 -m unittest discover -s .claude/tests -v
"""
import http.server
import importlib.util
import json
import pathlib
import shutil
import tempfile
import threading
import unittest

HOOKS = pathlib.Path(__file__).resolve().parents[1] / 'hooks'
spec = importlib.util.spec_from_file_location('remind_me_capture', HOOKS / 'remind-me-capture.py')
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)


class FakeServer(http.server.BaseHTTPRequestHandler):
    """A remind-me MCP endpoint: session-managed, answering tools/call as an event stream."""
    calls = []
    fail_tool = False

    def do_POST(self):
        message = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        FakeServer.calls.append((dict(self.headers), message))
        if self.headers.get('Authorization') != 'Bearer t0ken':
            self.send_response(401)
            self.end_headers()
            return
        if 'id' not in message:
            self.send_response(202)
            self.end_headers()
            return
        if message['method'] == 'initialize':
            body, kind = json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': {}}), 'application/json'
        else:
            result = {'isError': FakeServer.fail_tool, 'content': [{'type': 'text', 'text': 'ok'}]}
            reply = json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': result})
            body, kind = f'event: message\ndata: {reply}\n\n', 'text/event-stream'
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Mcp-Session-Id', 'sess-1')
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *args):
        pass


class Run(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.transcript = self.tmp / 's.jsonl'
        self.transcript.write_text('{"type":"user","message":{"role":"user","content":"hi"}}\n')
        self.server = http.server.HTTPServer(('127.0.0.1', 0), FakeServer)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        FakeServer.calls, FakeServer.fail_tool = [], False
        self.env = {'CLAUDE_CODE_REMOTE': 'true', 'REMIND_ME_TOKEN': 't0ken',
                    'REMIND_ME_URL': f'http://127.0.0.1:{self.server.server_port}/mcp'}

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def payload(self, event='Stop', **extra):
        return {'hook_event_name': event, 'session_id': 'cloud-1', 'cwd': str(self.tmp),
                'transcript_path': str(self.transcript), **extra}

    def run_hook(self, payload, now=1000.0, env=None):
        return hook.run(payload, self.env if env is None else env, now, self.tmp / 'state')

    def test_captures_through_the_mcp_handshake(self):
        self.assertEqual(self.run_hook(self.payload()), 'captured')
        methods = [m.get('method') for _, m in FakeServer.calls]
        self.assertEqual(methods, ['initialize', 'notifications/initialized', 'tools/call'])
        headers, call = FakeServer.calls[-1]
        self.assertEqual(headers.get('Mcp-Session-Id'), 'sess-1')
        args = call['params']['arguments']
        self.assertEqual(call['params']['name'], 'remind_me_capture_transcript')
        self.assertEqual((args['session_id'], args['reason']), ('cloud-1', 'stop'))
        self.assertEqual(args['transcript'], self.transcript.read_text())
        self.assertEqual(args['cwd'], str(self.tmp))

    def test_off_outside_a_cloud_session_or_without_config(self):
        for missing in ('CLAUDE_CODE_REMOTE', 'REMIND_ME_URL', 'REMIND_ME_TOKEN'):
            env = {k: v for k, v in self.env.items() if k != missing}
            self.assertEqual(self.run_hook(self.payload(), env=env), 'off')
        self.assertEqual(FakeServer.calls, [])

    def test_stop_is_throttled_and_skipped_when_reentrant_but_precompact_is_not(self):
        self.assertEqual(self.run_hook(self.payload()), 'captured')
        self.assertEqual(self.run_hook(self.payload(), now=1030.0), 'throttled')
        self.assertEqual(self.run_hook(self.payload('PreCompact'), now=1031.0), 'captured')
        self.assertEqual(FakeServer.calls[-1][1]['params']['arguments']['reason'], 'precompact')
        self.assertEqual(self.run_hook(self.payload(stop_hook_active=True), now=2000.0), 're-entrant')
        self.assertEqual(self.run_hook(self.payload(), now=1061.0), 'captured')

    def test_a_tool_error_is_raised_for_main_to_report(self):
        FakeServer.fail_tool = True
        with self.assertRaises(RuntimeError):
            self.run_hook(self.payload())

    def test_a_missing_transcript_captures_nothing(self):
        payload = self.payload(transcript_path=str(self.tmp / 'gone.jsonl'))
        self.assertEqual(self.run_hook(payload), 'nothing to capture')


class Helpers(unittest.TestCase):
    def test_tail_keeps_whole_newest_lines(self):
        raw = b'aaaa\nbbbb\ncccc\n'
        self.assertEqual(hook.tail(raw, 100), raw)
        self.assertEqual(hook.tail(raw, 8), b'cccc\n')
        self.assertEqual(hook.tail(b'x' * 10, 4), b'')

    def test_remote_credentials_are_stripped(self):
        self.assertEqual(hook.without_userinfo('http://proxy:s3cret@127.0.0.1:9/git/o/r'),
                         'http://127.0.0.1:9/git/o/r')
        self.assertEqual(hook.without_userinfo('git@github.com:o/r.git'), 'git@github.com:o/r.git')
        self.assertIsNone(hook.without_userinfo(None))

    def test_reply_to_reads_plain_and_event_stream_bodies(self):
        reply = {'jsonrpc': '2.0', 'id': 2, 'result': {}}
        self.assertEqual(hook.reply_to(2, json.dumps(reply), False), reply)
        stream = f'data: {json.dumps({"id": 1})}\n\ndata: {json.dumps(reply)}\n'
        self.assertEqual(hook.reply_to(2, stream, True), reply)
        self.assertIsNone(hook.reply_to(3, stream, True))


if __name__ == '__main__':
    unittest.main()

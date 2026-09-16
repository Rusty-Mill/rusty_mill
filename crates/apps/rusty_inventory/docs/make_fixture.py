#!/usr/bin/env python3
"""Build a fixture machine with all six tools installed and enough history
that the on-device semantic model has something to train on."""
import json, os, sqlite3, sys, time, shutil, random

HOME = sys.argv[1]
shutil.rmtree(HOME, ignore_errors=True)
now = int(time.time())
random.seed(7)

TOPICS = [
    ("kubernetes", "pod stuck terminating kubectl describe namespace cluster node drain evict finalizer",
     "container stuck terminating kubectl describe namespace cluster node drain evict finalizer"),
    ("postgres", "postgres index vacuum analyze planner query table sequential scan explain buffers",
     "database slow query planner statistics autovacuum bloat index table scan"),
    ("auth", "auth middleware session token refresh cookie hook shared react provider",
     "authentication login session jwt refresh token middleware guard route"),
    ("build", "webpack bundle build failing module resolve typescript config compile error",
     "vite build broken module resolution tsconfig paths compile failure"),
]

def w(path, body):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(body)

# ---- Claude Code -----------------------------------------------------------
for i in range(18):
    topic, a, b = TOPICS[i % len(TOPICS)]
    ts = now - (i + 1) * 3600
    lines = [
        json.dumps({"type": "user", "sessionId": f"cc-{i}", "cwd": f"/work/{topic}",
                    "gitBranch": "main" if i % 2 else "feat/x", "timestamp": ts,
                    "message": {"role": "user", "content": f"{a} — issue {i}"}}),
        json.dumps({"type": "assistant", "timestamp": ts + 120,
                    "message": {"role": "assistant",
                                "content": [{"type": "text", "text": f"{a}\n\n```bash\nkubectl get pods -A\n```"}]}}),
    ]
    w(f"{HOME}/.claude/projects/-work-{topic}/cc-{i}.jsonl", "\n".join(lines))

# One recognisable conversation from the product's own screenshot.
w(f"{HOME}/.claude/projects/-work-ui/cc-icons.jsonl", "\n".join([
    json.dumps({"type": "user", "sessionId": "cc-icons", "cwd": "/work/ui", "gitBranch": "main",
                "timestamp": now - 300,
                "message": {"role": "user", "content": "Monochromatic design with SVG icons"}}),
    json.dumps({"type": "assistant", "timestamp": now - 240,
                "message": {"role": "assistant", "content": [
                    {"type": "text", "text": "Use currentColor so the icons inherit the text colour."}]}}),
]))

# ---- Codex -----------------------------------------------------------------
for i in range(14):
    topic, a, b = TOPICS[i % len(TOPICS)]
    ts = now - (i + 1) * 7200
    lines = [
        json.dumps({"id": f"codex-{i}", "timestamp": ts, "cwd": f"/srv/{topic}"}),
        json.dumps({"type": "message", "role": "system", "content": "You are Codex. Follow the rules."}),
        json.dumps({"type": "response_item", "payload": {"type": "message", "role": "user",
                    "content": [{"type": "input_text", "text": f"{b} — case {i}"}]}, "timestamp": ts}),
        json.dumps({"type": "response_item", "payload": {"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": b}]}, "timestamp": ts + 300}),
    ]
    w(f"{HOME}/.codex/sessions/2026/08/05/rollout-{i}.jsonl", "\n".join(lines))

w(f"{HOME}/.codex/sessions/2026/08/05/rollout-pg.jsonl", "\n".join([
    json.dumps({"id": "codex-pg", "timestamp": now - 36000, "cwd": "/srv/search"}),
    json.dumps({"type": "message", "role": "user", "timestamp": now - 36000,
                "content": [{"type": "input_text", "text": "Postgres index tuning for the search table"}]}),
    json.dumps({"type": "message", "role": "assistant", "timestamp": now - 35000,
                "content": [{"type": "output_text", "text": "Add a partial index on updated_at."}]}),
]))

# ---- VS Code forks ---------------------------------------------------------
def vscdb(path, rows):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    c = sqlite3.connect(path)
    c.execute("CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value BLOB)")
    for k, v in rows:
        c.execute("INSERT INTO ItemTable VALUES (?,?)", (k, v))
    c.commit(); c.close()

tabs = []
for i in range(10):
    topic, a, b = TOPICS[i % len(TOPICS)]
    tabs.append({"tabId": f"cursor-{i}", "chatTitle": f"{topic} question {i}",
                 "lastUpdatedAt": (now - (i + 1) * 5400) * 1000,
                 "bubbles": [{"type": 1, "text": f"{a} — thread {i}"},
                             {"type": 2, "text": b}]})
tabs.append({"tabId": "cursor-git", "chatTitle": "Git remote setup and waitlist",
             "lastUpdatedAt": (now - 1860) * 1000,
             "bubbles": [{"type": 1, "text": "how do I add a git remote for the waitlist repo"},
                         {"type": 2, "text": "git remote add origin <url> then push -u"}]})
vscdb(f"{HOME}/.config/Cursor/User/globalStorage/state.vscdb",
      [("workbench.panel.aichat.view.aichat.chatdata", json.dumps({"tabs": tabs}))])

kiro = []
for i in range(8):
    topic, a, b = TOPICS[i % len(TOPICS)]
    kiro.append({"sessionId": f"kiro-{i}", "lastUpdatedAt": (now - (i + 1) * 9000) * 1000,
                 "requests": [{"message": {"text": f"{b} — kiro {i}"},
                               "response": [{"value": a}]}]})
kiro.append({"sessionId": "kiro-nodered", "lastUpdatedAt": (now - 39600) * 1000,
             "requests": [{"message": {"text": "Node-RED instance 404 error"},
                           "response": [{"value": "Check the httpAdminRoot base path."}]}]})
vscdb(f"{HOME}/.config/Kiro/User/globalStorage/state.vscdb",
      [("chat.sessions", json.dumps({"sessions": kiro}))])

anti = []
for i in range(7):
    topic, a, b = TOPICS[i % len(TOPICS)]
    anti.append({"id": f"anti-{i}", "lastUpdatedAt": (now - (i + 1) * 12000) * 1000,
                 "messages": [{"role": "user", "content": f"{a} — ag {i}"},
                              {"role": "assistant", "content": b}]})
anti.append({"id": "anti-trunc", "lastUpdatedAt": (now - 172800) * 1000,
             "messages": [{"role": "user", "content": "Antigravity truncates tool arguments at 256 bytes"},
                          {"role": "assistant", "content": "Known limit in the tool bridge."}]})
vscdb(f"{HOME}/.config/Antigravity/User/globalStorage/state.vscdb",
      [("agent.store", json.dumps({"threads": anti}))])

# ---- Zed -------------------------------------------------------------------
zed_path = f"{HOME}/.local/share/zed/threads/threads.db"
os.makedirs(os.path.dirname(zed_path), exist_ok=True)
c = sqlite3.connect(zed_path)
c.execute("CREATE TABLE threads(id TEXT PRIMARY KEY, summary TEXT, updated_at TEXT, data BLOB)")
rows = []
for i in range(9):
    topic, a, b = TOPICS[i % len(TOPICS)]
    rows.append((f"zed-{i}", f"{topic} thread {i}", now - (i + 1) * 11000,
                 json.dumps({"project_path": f"/work/{topic}", "git_branch": "main",
                             "messages": [{"role": "user", "segments": [{"type": "text", "text": f"{b} — zed {i}"}]},
                                          {"role": "assistant", "segments": [{"type": "text", "text": a}]}]})))
rows.append(("zed-auth", "Refactor the auth middleware into a shared hook", now - 36000,
             json.dumps({"project_path": "/work/web", "git_branch": "feat/auth",
                         "messages": [{"role": "user", "segments": [{"type": "text", "text": "refactor the auth middleware into a shared hook"}]},
                                      {"role": "assistant", "segments": [{"type": "text", "text": "Extracted useAuth into a provider."}]}]})))
c.executemany("INSERT INTO threads VALUES (?,?,?,?)", rows)
c.commit(); c.close()

print(f"fixture home: {HOME}")

// No logic here: render what the backend says, send the user's click back.
// Text is always set with textContent, never as HTML.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const LABEL = { absent: "not enabled", enabled: "enabled", stale: "stale", edited: "edited", foreign: "foreign" };
let pending = null; // { entry, enable } shown in the dialog

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function showError(e) {
  $("error").textContent = String(e);
  $("error").hidden = false;
}

function badge(state) {
  return el("span", `badge ${state}`, LABEL[state] ?? state);
}

// One button per row: Enable when nothing is enabled, else Disable.
function actionFor(row) {
  const any = row.cells.some((c) => c.state === "enabled");
  const btn = el("button", "", any ? "Disable…" : "Enable…");
  btn.addEventListener("click", () => preview(row.id, !any));
  return btn;
}

function render(view) {
  const head = $("grid").tHead;
  head.replaceChildren();
  const hr = head.insertRow();
  ["Skill", ...view.tools, ""].forEach((t) => hr.appendChild(el("th", "", t)));
  const body = $("grid").tBodies[0];
  body.replaceChildren();
  for (const row of view.rows) {
    const tr = body.insertRow();
    const name = tr.insertCell();
    name.appendChild(document.createTextNode(row.id));
    name.appendChild(el("small", "", row.source));
    for (const c of row.cells) tr.insertCell().appendChild(badge(c.state));
    tr.insertCell().appendChild(actionFor(row));
  }
  $("count").textContent = `${view.rows.length} skills`;
  const skipped = view.skipped.map((s) => `${s.path}: ${s.reason}`);
  $("skipped").textContent = skipped.length ? `Skipped: ${skipped.join("; ")}` : "";
  $("skipped").hidden = skipped.length === 0;
}

async function refresh() {
  try {
    $("error").hidden = true;
    render(await invoke("list"));
  } catch (e) {
    showError(e);
  }
}

async function preview(entry, enable) {
  try {
    const plans = await invoke("plan_change", { entry, enable });
    pending = { entry, enable };
    $("plan-title").textContent = `${enable ? "Enable" : "Disable"} ${entry}`;
    const body = $("plan-body");
    body.replaceChildren();
    let changes = 0;
    let refused = 0;
    for (const p of plans) {
      const box = el("div", "tool");
      box.appendChild(el("div", "dim", p.tool));
      if (p.noop) box.appendChild(el("div", "dim", "nothing to do"));
      for (const a of p.actions) {
        const r = el("div", "row");
        r.appendChild(el("span", `kind ${a.kind}`, a.kind));
        const arrow = a.source ? ` → ${a.source} (${a.method})` : a.reason ? ` ${a.reason}` : "";
        r.appendChild(el("span", "", `${a.path}${arrow}`));
        box.appendChild(r);
        if (a.kind === "refuse") refused += 1;
        else if (!p.refused) changes += 1;
      }
      body.appendChild(box);
    }
    const n = (k, w) => `${k} ${w}${k === 1 ? "" : "s"}`;
    $("plan-summary").textContent = `${n(changes, "change")}, ${refused} refused. A refused tool is left untouched.`;
    $("plan-apply").disabled = changes === 0;
    $("plan-apply").textContent = `Apply ${n(changes, "change")}`;
    $("plan").showModal();
  } catch (e) {
    showError(e);
  }
}

$("plan-cancel").addEventListener("click", () => $("plan").close());
$("plan-apply").addEventListener("click", async () => {
  const { entry, enable } = pending;
  $("plan").close();
  try {
    await invoke("apply_change", { entry, enable });
  } catch (e) {
    showError(e);
  }
  refresh();
});

refresh();

// Central reactive state: the `rk://` event stream drives fine-grained signals so
// only the streaming spans re-render, not the whole turn list (PRD 08).

import { createSignal, type Accessor } from "solid-js";
import { createStore, produce } from "solid-js/store";
import {
  asBoundaryError,
  ipc,
  on,
} from "./ipc";
import { EVENT } from "./contract";
import type {
  ApprovalRequest,
  BoundaryError,
  EntropyAudit,
  MhirReport,
  SessionConfig,
  TokenBudget,
  ToolEvent,
  TurnCompleteEvent,
} from "./contract";

/// One completed turn rendered as a `<TurnCard>`.
export interface Turn {
  id: string;
  user_message: string;
  reply: string;
  verified: boolean;
  limits: string;
  tool_events: ToolEvent[];
  ts: number;
}

/// Which context-panel tab is focused, and whether the human pinned it.
export type ContextTab = "terminal" | "editor" | "git" | "memory" | "web";

// ---- Signals & stores ----

const [turns, setTurns] = createStore<Turn[]>([]);
const [pendingTokens, setPendingTokens] = createSignal("");
const [pendingTools, setPendingTools] = createStore<ToolEvent[]>([]);
const [locked, setLocked] = createSignal(false);
const [approval, setApproval] = createSignal<ApprovalRequest | null>(null);
const [planExit, setPlanExit] = createSignal<string | null>(null);
const [error, setError] = createSignal<BoundaryError | null>(null);
const [entropy, setEntropy] = createStore<EntropyAudit[]>([]);
const [mhir, setMhir] = createSignal<MhirReport | null>(null);
const [budget, setBudget] = createSignal<TokenBudget | null>(null);
const [config, setConfig] = createSignal<SessionConfig | null>(null);
const [activeTab, setActiveTab] = createSignal<ContextTab>("terminal");
const [pinned, setPinned] = createSignal(false);
const [terminalChunks, setTerminalChunks] = createStore<string[]>([]);

let pendingUserMessage = "";

/// Auto-focus a context tab from a fired tool (PRD 08 table), unless pinned.
function focusForTool(name: string) {
  if (pinned()) return;
  const tab: ContextTab | null =
    name === "bash" || name === "bash_background"
      ? "terminal"
      : name === "read_file" ||
          name === "edit_file" ||
          name === "write_file" ||
          name === "glob" ||
          name === "list_directory"
        ? "editor"
        : name === "web_fetch" || name === "web_search"
          ? "web"
          : null;
  if (tab) setActiveTab(tab);
}

/// Refresh the header metrics after a turn settles.
async function refreshHeader() {
  try {
    setMhir(await ipc.mhir());
  } catch (_) {
    /* header metrics are best-effort */
  }
  try {
    setBudget(await ipc.tokenBudget());
  } catch (_) {
    /* */
  }
  try {
    setConfig(await ipc.config());
  } catch (_) {
    /* */
  }
}

/// Subscribe to the canonical event stream. Returns an unlisten-all function.
export async function wireEvents(): Promise<() => void> {
  const unlisteners = await Promise.all([
    on<{ turn_id: string }>(EVENT.TURN_START, () => {
      setLocked(true);
      setError(null);
      setPendingTokens("");
      setPendingTools([]);
    }),
    on<string>(EVENT.TOKEN, (chunk) => setPendingTokens((t) => t + chunk)),
    on<ToolEvent>(EVENT.TOOL_EVENT, (ev) => {
      setPendingTools(produce((list) => list.push(ev)));
      focusForTool(ev.name);
    }),
    on<TurnCompleteEvent>(EVENT.TURN_COMPLETE, (payload) => {
      const turn: Turn = {
        id: payload.turn_id,
        user_message: pendingUserMessage,
        reply: payload.reply,
        verified: payload.verified,
        limits: payload.limits,
        tool_events: [...pendingTools],
        ts: Date.now(),
      };
      setTurns(produce((list) => list.push(turn)));
      setPendingTokens("");
      setPendingTools([]);
      setLocked(false);
      void refreshHeader();
    }),
    on<ApprovalRequest>(EVENT.APPROVAL_REQUEST, (req) => setApproval(req)),
    on<string>(EVENT.PLAN_EXIT, (plan) => setPlanExit(plan)),
    on<EntropyAudit>(EVENT.ENTROPY, (audit) =>
      setEntropy(produce((list) => list.push(audit))),
    ),
    on<string>(EVENT.BASH_OUTPUT, (chunk) =>
      setTerminalChunks(produce((list) => list.push(chunk))),
    ),
    on<unknown>(EVENT.CONSOLIDATION, () => {
      if (!pinned()) setActiveTab("memory");
    }),
  ]);
  return () => unlisteners.forEach((u) => u());
}

/// Send a user turn. The success path is driven by `rk://turn_complete`; this
/// only needs to catch the boundary-error rejection and unlock the composer.
export async function sendTurn(message: string, attachments?: string[]) {
  if (!message.trim() || locked()) return;
  pendingUserMessage = message;
  setLocked(true);
  setError(null);
  try {
    await ipc.sessionSend(message, attachments);
  } catch (err) {
    setError(asBoundaryError(err));
    setLocked(false); // failed turns never emit rk://turn_complete
  }
}

/// Answer the in-flight approval request.
export async function respondApproval(approved: boolean, always = false) {
  setApproval(null);
  try {
    await ipc.approvalRespond(approved, always);
  } catch (err) {
    setError(asBoundaryError(err));
  }
}

/// Resolve a plan-mode proposal. Annotation is re-sent as a follow-up turn.
export async function resolvePlan(decision: "proceed" | "reject", note?: string) {
  const plan = planExit();
  setPlanExit(null);
  if (decision === "proceed") {
    await sendTurn("Proceed with the proposed plan.");
  } else if (note && note.trim()) {
    await sendTurn(note);
  }
  void plan;
}

export const store = {
  turns,
  pendingTokens,
  pendingTools,
  locked,
  approval,
  planExit,
  error,
  setError,
  entropy,
  mhir,
  budget,
  config,
  activeTab,
  setActiveTab,
  pinned,
  setPinned,
  terminalChunks,
  refreshHeader,
};

export type Store = typeof store;
export type { Accessor };

import { onCleanup, onMount, Show, createSignal } from "solid-js";
import { store, wireEvents } from "./store";
import Header from "./components/Header";
import SessionPanel from "./components/SessionPanel";
import ContextPanel from "./components/ContextPanel";
import Composer from "./components/Composer";
import ErrorBanner from "./components/ErrorBanner";
import Dashboard from "./components/Dashboard";
import Settings from "./components/Settings";
import { ResizableSplit } from "./components/ResizableSplit";

export default function App() {
  const [showDashboard, setShowDashboard] = createSignal(false);
  const [showSettings, setShowSettings] = createSignal(false);

  onMount(async () => {
    const unlisten = await wireEvents();
    void store.refreshHeader();
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "h") {
        e.preventDefault();
        setShowDashboard((v) => !v);
      }
      if (e.key === "Escape") {
        setShowDashboard(false);
        setShowSettings(false);
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      unlisten();
      window.removeEventListener("keydown", onKey);
    });
  });

  return (
    <div class="flex h-full flex-col">
      <Header
        onOpenDashboard={() => setShowDashboard(true)}
        onOpenSettings={() => setShowSettings(true)}
      />
      <div class="relative min-h-0 flex-1">
        <ResizableSplit
          left={<SessionPanel />}
          right={<ContextPanel />}
          initial={0.52}
          min={0.3}
          max={0.75}
        />
        <Show when={showDashboard()}>
          <Dashboard onClose={() => setShowDashboard(false)} />
        </Show>
        <Show when={showSettings()}>
          <Settings onClose={() => setShowSettings(false)} />
        </Show>
      </div>
      <ErrorBanner />
      <Composer />
    </div>
  );
}

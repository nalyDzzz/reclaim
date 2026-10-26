import { useState } from "react";
import { Sidebar, type Page } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Toasts } from "./components/Toasts";
import { AppsView } from "./features/apps/AppsView";
import { CleanupView } from "./features/cleanup/CleanupView";
import { DebloatView } from "./features/debloat/DebloatView";
import { StorageView } from "./features/storage/StorageView";
import { AppProvider } from "./state/app";

const VIEWS: Record<Page, React.ComponentType> = {
  apps: AppsView,
  storage: StorageView,
  cleanup: CleanupView,
  debloat: DebloatView,
};

export default function App() {
  const [page, setPage] = useState<Page>("apps");
  // Views mount on first visit and then stay mounted, so a finished scan or
  // a scrolled list is still there when you come back.
  const [visited, setVisited] = useState<Set<Page>>(() => new Set(["apps"]));

  const navigate = (next: Page) => {
    setPage(next);
    setVisited((v) => (v.has(next) ? v : new Set(v).add(next)));
  };

  return (
    <AppProvider>
      <div className="flex h-full flex-col">
        <TitleBar />
        <div className="flex min-h-0 flex-1">
          <Sidebar page={page} onNavigate={navigate} />
          <main className="relative min-w-0 flex-1 overflow-hidden rounded-tl-[var(--radius-panel)] border-t border-l border-line bg-panel [html.mica_&]:bg-panel/88">
            {(Object.keys(VIEWS) as Page[]).map((p) => {
              if (!visited.has(p)) return null;
              const View = VIEWS[p];
              return (
                <div key={p} className="absolute inset-0 flex flex-col" hidden={p !== page}>
                  <View />
                </div>
              );
            })}
          </main>
        </div>
      </div>
      <Toasts />
    </AppProvider>
  );
}

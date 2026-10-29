import { useEffect, useState } from "react";
import { Sidebar, type Page } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Toasts } from "./components/Toasts";
import { UpdateDialog } from "./components/UpdateDialog";
import { AppsView } from "./features/apps/AppsView";
import { CleanupView } from "./features/cleanup/CleanupView";
import { DebloatView } from "./features/debloat/DebloatView";
import { StorageView, type ScanRequest } from "./features/storage/StorageView";
import { api } from "./lib/api";
import { AppProvider } from "./state/app";
import { UpdatesProvider } from "./state/updates";

export default function App() {
  const [page, setPage] = useState<Page>("apps");
  // Views mount on first visit and then stay mounted, so a finished scan or
  // a scrolled list is still there when you come back.
  const [visited, setVisited] = useState<Set<Page>>(() => new Set(["apps"]));

  const navigate = (next: Page) => {
    setPage(next);
    setVisited((v) => (v.has(next) ? v : new Set(v).add(next)));
  };

  // Explorer's "Scan with Reclaim" menu, whether it started Reclaim or found
  // it open. Taking once the listener is up catches a request that arrived
  // while it was being set up.
  const [scanRequest, setScanRequest] = useState<ScanRequest | null>(null);
  useEffect(() => {
    const take = () =>
      api.takePendingScan().then((path) => {
        if (!path) return;
        navigate("storage");
        setScanRequest({ path });
      });
    const unlisten = api.onScanRequested(take);
    unlisten.then(take);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const views: Record<Page, React.ReactNode> = {
    apps: <AppsView />,
    storage: <StorageView request={scanRequest} />,
    cleanup: <CleanupView />,
    debloat: <DebloatView />,
  };

  return (
    <AppProvider>
      <UpdatesProvider>
        <div className="flex h-full flex-col">
          <TitleBar />
          <div className="flex min-h-0 flex-1">
            <Sidebar page={page} onNavigate={navigate} />
            <main className="relative min-w-0 flex-1 overflow-hidden rounded-tl-[var(--radius-panel)] border-t border-l border-line bg-panel [html.mica_&]:bg-panel/88">
              {(Object.keys(views) as Page[]).map((p) => {
                if (!visited.has(p)) return null;
                return (
                  <div key={p} className="absolute inset-0 flex flex-col" hidden={p !== page}>
                    {views[p]}
                  </div>
                );
              })}
            </main>
          </div>
        </div>
        <UpdateDialog />
        <Toasts />
      </UpdatesProvider>
    </AppProvider>
  );
}

import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { crustApi } from '../api/client';
import { useSessionStore } from '../stores/sessionStore';
import { useUiStore } from '../stores/uiStore';
import { ChatPanel } from './ChatPanel';
import { SessionSidebar } from './SessionSidebar';

export function AppShell() {
  const setSessions = useSessionStore((state) => state.setSessions);
  const sidebarOpen = useUiStore((state) => state.sidebarOpen);
  const setSidebarOpen = useUiStore((state) => state.setSidebarOpen);
  const sessionsQuery = useQuery({ queryKey: ['sessions'], queryFn: crustApi.listSessions });

  useEffect(() => {
    if (sessionsQuery.data) {
      setSessions(sessionsQuery.data);
    }
  }, [sessionsQuery.data, setSessions]);

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top_left,#263244_0,#10131a_34rem)] text-slate-100">
      <div className="mx-auto flex min-h-screen w-full max-w-7xl flex-col lg:flex-row">
        <SessionSidebar open={sidebarOpen} onClose={() => setSidebarOpen(false)} loading={sessionsQuery.isLoading} />
        <main className="flex min-h-screen flex-1 flex-col border-x border-white/10 bg-crust-ink/70 shadow-2xl shadow-black/40 backdrop-blur">
          <header className="flex items-center justify-between border-b border-white/10 px-4 py-3 lg:hidden">
            <div>
              <p className="text-xs uppercase tracking-[0.24em] text-crust-mint">Crust</p>
              <h1 className="font-semibold">React Console</h1>
            </div>
            <button className="rounded-lg border border-white/15 px-3 py-2 text-sm" onClick={() => setSidebarOpen(true)}>
              Sessions
            </button>
          </header>
          <ChatPanel />
        </main>
      </div>
    </div>
  );
}

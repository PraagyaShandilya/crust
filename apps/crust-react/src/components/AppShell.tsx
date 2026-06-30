import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { crustApi } from '../api/client';
import { useSessionStore } from '../stores/sessionStore';
import type { SessionSummary } from '../types/chat';
import { useUiStore } from '../stores/uiStore';
import { ChatPanel } from './ChatPanel';
import { SessionSidebar } from './SessionSidebar';
import { SettingsPanel } from './SettingsPanel';
import { SpacesDashboard } from './SpacesDashboard';
import { RightSidebar } from './RightSidebar';
import { ToastViewport } from './ToastViewport';
import { useToastStore } from '../stores/toastStore';

const NAV_TABS: { id: 'chat' | 'settings' | 'spaces'; label: string }[] = [
  { id: 'chat', label: 'Chat' },
  { id: 'spaces', label: 'Spaces' },
  { id: 'settings', label: 'Settings' },
];

export function AppShell() {
  const queryClient = useQueryClient();
  const setSessions = useSessionStore((state) => state.setSessions);
  const setActiveSession = useSessionStore((state) => state.setActiveSession);
  const sidebarOpen = useUiStore((state) => state.sidebarOpen);
  const setSidebarOpen = useUiStore((state) => state.setSidebarOpen);
  const activeView = useUiStore((state) => state.activeView);
  const setActiveView = useUiStore((state) => state.setActiveView);
  const notify = useToastStore((state) => state.notify);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  const sessionsQuery = useQuery({ queryKey: ['sessions'], queryFn: crustApi.listSessions });
  const createSession = useMutation({
    mutationFn: () => crustApi.createSession(),
    onSuccess: (session) => {
      queryClient.setQueryData<SessionSummary[]>(['sessions'], (sessions = []) => [session, ...sessions]);
      setSessions([session, ...(sessionsQuery.data ?? [])]);
      setActiveSession(session.id);
      setActiveView('chat');
    },
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to create session', 'error'),
  });

  useEffect(() => {
    if (sessionsQuery.data) {
      setSessions(sessionsQuery.data);
    }
  }, [sessionsQuery.data, setSessions]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        setPaletteOpen(true);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const commands = useMemo(() => [
    { label: 'New session', detail: 'Create and switch to a new chat', run: () => createSession.mutate() },
    { label: 'Open chat', detail: 'Switch to the chat console', run: () => setActiveView('chat') },
    { label: 'Open spaces', detail: 'Switch to workspaces', run: () => setActiveView('spaces') },
    { label: 'Open settings', detail: 'Switch to settings', run: () => setActiveView('settings') },
    { label: 'Toggle sessions', detail: 'Open or close session sidebar', run: () => setSidebarOpen(!sidebarOpen) },
  ], [createSession, setActiveView, setSidebarOpen, sidebarOpen]);

  const visibleCommands = commands.filter((command) => {
    const query = paletteQuery.trim().toLowerCase();
    return !query || command.label.toLowerCase().includes(query) || command.detail.toLowerCase().includes(query);
  });

  return (
    <div className="h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,rgba(255,106,0,0.18)_0,#050505_28rem)] text-crust-text">
      <div className="flex h-full w-full flex-col">
        <header className="flex items-center gap-2 border-b border-crust-mint/20 bg-crust-panel/80 px-4 py-2 backdrop-blur">
          <p className="mr-4 text-xs uppercase tracking-[0.28em] text-crust-mint">Crust</p>
          <nav className="flex items-center gap-1">
            {NAV_TABS.map((tab) => (
              <button
                key={tab.id}
                className={`rounded-lg px-3 py-1.5 text-sm transition ${
                  activeView === tab.id
                    ? 'bg-crust-mint/15 text-crust-mint'
                    : 'text-crust-text-muted hover:text-crust-text'
                }`}
                onClick={() => setActiveView(tab.id)}
              >
                {tab.label}
              </button>
            ))}
          </nav>
        </header>

        <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
          <SessionSidebar
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
            loading={sessionsQuery.isLoading || createSession.isPending}
            onCreate={() => createSession.mutate()}
          />
          <main className="flex min-h-0 flex-1 flex-col border-l border-crust-mint/20 bg-crust-ink/85 shadow-2xl shadow-crust-ember/10 backdrop-blur">
            <header className="flex items-center justify-between border-b border-crust-mint/20 px-4 py-3 lg:hidden">
              <div>
                <p className="text-xs uppercase tracking-[0.24em] text-crust-mint">Crust</p>
                <h1 className="font-semibold">React Console</h1>
              </div>
              <button className="rounded-lg border border-crust-mint/40 px-3 py-2 text-sm text-crust-mint" onClick={() => setSidebarOpen(true)}>
                Sessions
              </button>
            </header>
            {activeView === 'chat' && <ChatPanel />}
            {activeView === 'settings' && <SettingsPanel />}
            {activeView === 'spaces' && <SpacesDashboard />}
          </main>
          <RightSidebar />
        </div>
      </div>
      {paletteOpen && (
        <div className="fixed inset-0 z-50 bg-black/50 px-4 pt-24 backdrop-blur-sm" onClick={() => setPaletteOpen(false)}>
          <div className="mx-auto max-w-lg overflow-hidden rounded-2xl border border-crust-mint/25 bg-crust-coal shadow-2xl shadow-crust-ember/20" onClick={(event) => event.stopPropagation()}>
            <input
              autoFocus
              className="w-full border-b border-crust-mint/15 bg-transparent px-4 py-3 text-sm text-crust-text outline-none placeholder:text-crust-text-muted/35"
              placeholder="Run command..."
              value={paletteQuery}
              onChange={(event) => setPaletteQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  setPaletteOpen(false);
                }
                if (event.key === 'Enter' && visibleCommands[0]) {
                  visibleCommands[0].run();
                  setPaletteOpen(false);
                  setPaletteQuery('');
                }
              }}
            />
            <div className="max-h-72 overflow-y-auto p-2">
              {visibleCommands.map((command) => (
                <button
                  key={command.label}
                  className="flex w-full flex-col rounded-lg px-3 py-2 text-left hover:bg-crust-mint/10"
                  onClick={() => {
                    command.run();
                    setPaletteOpen(false);
                    setPaletteQuery('');
                  }}
                >
                  <span className="text-sm text-crust-text">{command.label}</span>
                  <span className="text-xs text-crust-text-muted/60">{command.detail}</span>
                </button>
              ))}
              {visibleCommands.length === 0 && <p className="px-3 py-4 text-sm text-crust-text-muted/60">No commands found.</p>}
            </div>
          </div>
        </div>
      )}
      <ToastViewport />
    </div>
  );
}

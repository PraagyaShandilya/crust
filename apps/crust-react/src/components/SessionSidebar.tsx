import { useSessionStore } from '../stores/sessionStore';

interface SessionSidebarProps {
  open: boolean;
  loading: boolean;
  onClose: () => void;
}

export function SessionSidebar({ open, loading, onClose }: SessionSidebarProps) {
  const sessions = useSessionStore((state) => state.sessions);
  const activeSessionId = useSessionStore((state) => state.activeSessionId);
  const setActiveSession = useSessionStore((state) => state.setActiveSession);

  return (
    <aside
      className={`fixed inset-y-0 left-0 z-20 w-80 border-r border-white/10 bg-crust-panel p-4 transition-transform lg:static lg:translate-x-0 ${
        open ? 'translate-x-0' : '-translate-x-full'
      }`}
    >
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <p className="text-xs uppercase tracking-[0.28em] text-crust-mint">Crust</p>
          <h1 className="mt-1 text-xl font-semibold">Sessions</h1>
        </div>
        <button className="rounded-md border border-white/15 px-2 py-1 text-sm text-slate-300 lg:hidden" onClick={onClose}>
          Close
        </button>
      </div>

      <button className="mb-4 w-full rounded-xl border border-dashed border-crust-mint/50 px-4 py-3 text-left text-sm text-crust-mint">
        New session placeholder
      </button>

      <div className="space-y-2">
        {loading && <p className="text-sm text-slate-400">Loading sessions...</p>}
        {sessions.map((session) => (
          <button
            key={session.id}
            className={`w-full rounded-xl border p-3 text-left transition ${
              activeSessionId === session.id ? 'border-crust-mint/70 bg-crust-mint/10' : 'border-white/10 bg-white/[0.03] hover:bg-white/[0.06]'
            }`}
            onClick={() => {
              setActiveSession(session.id);
              onClose();
            }}
          >
            <div className="flex items-center justify-between gap-3">
              <span className="font-medium">{session.title}</span>
              <span className="rounded-full bg-white/10 px-2 py-0.5 text-xs text-slate-300">{session.status}</span>
            </div>
            <p className="mt-2 text-xs text-slate-400">Updated {new Date(session.updatedAt).toLocaleString()}</p>
          </button>
        ))}
      </div>
    </aside>
  );
}

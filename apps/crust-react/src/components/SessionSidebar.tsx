import { useMemo, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { crustApi } from '../api/client';
import { useSessionStore } from '../stores/sessionStore';
import { useUiStore } from '../stores/uiStore';
import { useToastStore } from '../stores/toastStore';
import type { SessionSummary } from '../types/chat';
import { coreColorClass } from '../utils/coreColors';

const PINNED_SESSION_STORAGE_KEY = 'crust:pinned-sessions';
const SIDEBAR_WIDTH_STORAGE_KEY = 'crust:left-sidebar-width';
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 420;

function formatCoreKind(kind: string): string {
  return kind.replace(/_/g, ' ');
}

interface SessionSidebarProps {
  open: boolean;
  loading: boolean;
  onClose: () => void;
  onCreate: () => void;
}

export function SessionSidebar({ open, loading, onClose, onCreate }: SessionSidebarProps) {
  const sessions = useSessionStore((state) => state.sessions);
  const activeSessionId = useSessionStore((state) => state.activeSessionId);
  const setActiveSession = useSessionStore((state) => state.setActiveSession);
  const setActiveView = useUiStore((state) => state.setActiveView);
  const notify = useToastStore((state) => state.notify);
  const removeSession = useSessionStore((state) => state.removeSession);
  const updateSession = useSessionStore((state) => state.updateSession);
  const queryClient = useQueryClient();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState('');
  const [highlightedId, setHighlightedId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [sidebarWidth, setSidebarWidth] = useState(() => Number(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)) || 288);
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem(PINNED_SESSION_STORAGE_KEY) ?? '[]'));
    } catch {
      return new Set();
    }
  });

  const visibleSessions = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    const filtered = query
      ? sessions.filter((session) => session.title.toLowerCase().includes(query) || (session.coreKind ?? 'general').toLowerCase().includes(query))
      : sessions;

    return [...filtered].sort((a, b) => Number(pinnedIds.has(b.id)) - Number(pinnedIds.has(a.id)));
  }, [pinnedIds, searchQuery, sessions]);

  const selectSession = (sessionId: string) => {
    setActiveSession(sessionId);
    setActiveView('chat');
    setHighlightedId(sessionId);
    onClose();
  };

  const moveHighlight = (direction: 1 | -1) => {
    if (visibleSessions.length === 0) {
      return;
    }

    const currentId = highlightedId ?? activeSessionId;
    const currentIndex = currentId ? visibleSessions.findIndex((session) => session.id === currentId) : -1;
    const nextIndex = currentIndex === -1
      ? direction === 1 ? 0 : visibleSessions.length - 1
      : (currentIndex + direction + visibleSessions.length) % visibleSessions.length;

    setHighlightedId(visibleSessions[nextIndex].id);
  };

  const togglePinned = (sessionId: string) => {
    setPinnedIds((current) => {
      const next = new Set(current);
      if (next.has(sessionId)) {
        next.delete(sessionId);
      } else {
        next.add(sessionId);
      }
      localStorage.setItem(PINNED_SESSION_STORAGE_KEY, JSON.stringify([...next]));
      return next;
    });
  };

  const startResize = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    const onMouseMove = (moveEvent: globalThis.MouseEvent) => {
      const nextWidth = Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, moveEvent.clientX));
      setSidebarWidth(nextWidth);
      localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(nextWidth));
    };
    const onMouseUp = () => {
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    };
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  };

  const renameMutation = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) => crustApi.renameSession(id, name),
    onSuccess: (session: SessionSummary, vars) => {
      updateSession(vars.id, { title: session.title, updatedAt: session.updatedAt });
      queryClient.setQueryData<SessionSummary[]>(['sessions'], (prev = []) =>
        prev.map((s) => (s.id === vars.id ? { ...s, title: session.title } : s)),
      );
      setEditingId(null);
    },
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to rename session', 'error'),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crustApi.deleteSession(id),
    onSuccess: (_void, id) => {
      removeSession(id);
      queryClient.setQueryData<SessionSummary[]>(['sessions'], (prev = []) =>
        prev.filter((s) => s.id !== id),
      );
    },
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to delete session', 'error'),
  });

  return (
    <aside
      className={`fixed inset-y-0 left-0 z-20 flex flex-col border-r border-crust-mint/20 bg-crust-panel p-3 shadow-2xl shadow-crust-ember/10 transition-transform lg:static lg:h-full lg:translate-x-0 ${
        open ? 'translate-x-0' : '-translate-x-full'
      }`}
      style={{ width: sidebarWidth }}
    >
      <div className="mb-4 flex items-center justify-between gap-4">
        <div>
          <p className="text-[0.65rem] uppercase tracking-[0.28em] text-crust-mint">Crust</p>
          <h1 className="mt-0.5 text-lg font-semibold text-crust-text">Sessions</h1>
        </div>
        <button className="rounded-md border border-crust-mint/30 px-2 py-1 text-xs text-crust-text-muted lg:hidden" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="mb-3 flex items-center gap-2">
        <input
          className="min-w-0 flex-1 rounded-lg border border-crust-mint/20 bg-crust-coal px-3 py-2 text-sm text-crust-text outline-none placeholder:text-crust-text-muted/35 focus:border-crust-mint/60"
          placeholder="Search sessions..."
          value={searchQuery}
          onChange={(event) => setSearchQuery(event.target.value)}
        />
        <button
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-crust-mint/60 bg-crust-mint/5 text-lg leading-none text-crust-mint shadow-inner shadow-crust-ember/10 transition hover:bg-crust-mint/10 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={loading}
          onClick={onCreate}
          title="New session"
          aria-label="New session"
        >
          +
        </button>
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto pr-0.5"
        onKeyDown={(e) => {
          if (editingId) {
            return;
          }

          if (e.key === 'ArrowDown') {
            e.preventDefault();
            moveHighlight(1);
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            moveHighlight(-1);
          } else if (e.key === 'Enter' && highlightedId) {
            e.preventDefault();
            selectSession(highlightedId);
          }
        }}
      >
        {loading && <p className="px-1 py-2 text-sm text-crust-text-subtle/70">Loading...</p>}
        {!loading && visibleSessions.length === 0 && <p className="px-1 py-2 text-sm text-crust-text-subtle/70">No sessions found.</p>}
        {visibleSessions.map((session) => {
          const colors = coreColorClass(session.coreKind ?? 'general');
          const isActive = activeSessionId === session.id;
          const isHighlighted = highlightedId === session.id;
          const isPinned = pinnedIds.has(session.id);
          return (
          <div
            key={session.id}
            role="button"
            tabIndex={0}
            aria-current={isActive ? 'page' : undefined}
            className={`group relative mb-1 overflow-hidden rounded-lg border py-1 pl-2.5 pr-2 text-left transition focus:outline-none focus:ring-1 focus:ring-crust-mint/60 ${
              isActive
                ? 'border-crust-mint/70 bg-crust-mint/10'
                : isHighlighted
                  ? 'border-crust-mint/45 bg-crust-mint/[0.08]'
                : 'border-transparent hover:border-crust-mint/30 hover:bg-crust-mint/[0.06]'
            }`}
            onClick={() => selectSession(session.id)}
            onFocus={() => setHighlightedId(session.id)}
          >
            <div className={`absolute inset-y-0 left-0 w-0.5 ${colors.bg.replace('/10', '/60')}`} />
            {editingId === session.id ? (
              <div className="flex items-center gap-1.5 py-0.5">
                <input
                  autoFocus
                  className="min-w-0 flex-1 rounded border border-crust-mint/50 bg-crust-ink px-2 py-0.5 text-sm text-crust-text outline-none focus:border-crust-mint"
                  value={draftName}
                  onChange={(e) => setDraftName(e.target.value)}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === 'Enter' && draftName.trim()) {
                      renameMutation.mutate({ id: session.id, name: draftName.trim() });
                    } else if (e.key === 'Escape') {
                      setEditingId(null);
                    }
                  }}
                />
                <button
                  className="shrink-0 rounded p-1 text-crust-mint hover:bg-crust-mint/15"
                  disabled={!draftName.trim() || renameMutation.isPending}
                  onClick={(e) => {
                    e.stopPropagation();
                    renameMutation.mutate({ id: session.id, name: draftName.trim() });
                  }}
                  title="Save"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                </button>
                <button
                  className="shrink-0 rounded p-1 text-crust-text-muted hover:bg-crust-mint/10"
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditingId(null);
                  }}
                  title="Cancel"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                </button>
              </div>
            ) : (
              <div className="flex items-center gap-1.5">
                <button
                  className="min-w-0 flex-1 text-left"
                  onClick={(e) => {
                    e.stopPropagation();
                    selectSession(session.id);
                  }}
                >
                  <div className="flex items-center gap-2">
                    <span className={`truncate text-xs ${isActive ? 'font-medium text-crust-text' : 'text-crust-text-muted'}`}>
                      {session.title}
                    </span>
                    {isActive && (
                      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-crust-mint" />
                    )}
                  </div>
                  <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
                    <span className={`inline-flex max-w-[7rem] truncate rounded-full px-1.5 py-px text-[0.58rem] capitalize ${colors.badge}`}>
                      {formatCoreKind(session.coreKind ?? 'general')}
                    </span>
                    <span className="truncate text-[0.62rem] text-crust-text-muted/35">
                      {new Date(session.updatedAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                    </span>
                  </div>
                </button>
                <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100">
                  <button
                    className={`rounded p-1 ${isPinned ? 'text-crust-amber hover:bg-crust-amber/15' : 'text-crust-text-muted/60 hover:bg-crust-mint/15 hover:text-crust-mint'}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      togglePinned(session.id);
                    }}
                    title={isPinned ? 'Unpin' : 'Pin'}
                    aria-label={isPinned ? 'Unpin session' : 'Pin session'}
                    aria-pressed={isPinned}
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill={isPinned ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 17v5"/><path d="M5 17h14"/><path d="M7 9V4h10v5l2 4H5l2-4z"/></svg>
                  </button>
                  <button
                    className="rounded p-1 text-crust-text-muted/60 hover:bg-crust-mint/15 hover:text-crust-mint"
                    onClick={(e) => {
                      e.stopPropagation();
                      setEditingId(session.id);
                      setDraftName(session.title);
                    }}
                    title="Rename"
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
                  </button>
                  <button
                    className="rounded p-1 text-crust-text-muted/60 hover:bg-crust-ember/15 hover:text-crust-ember"
                    disabled={deleteMutation.isPending}
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteMutation.mutate(session.id);
                    }}
                    title="Delete"
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                  </button>
                </div>
              </div>
            )}
          </div>
          );
        })}
      </div>
      <div
        className="absolute inset-y-0 right-0 hidden w-1 cursor-col-resize bg-transparent transition hover:bg-crust-mint/40 lg:block"
        onMouseDown={startResize}
        title="Resize sidebar"
      />
    </aside>
  );
}

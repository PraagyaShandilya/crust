import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { crustApi } from '../api/client';
import { useSessionStore } from '../stores/sessionStore';
import { useToastStore } from '../stores/toastStore';
import { Composer } from './Composer';
import { MessageList } from './MessageList';
import type { ChatEvent, ChatMessage, ContextInfo } from '../types/chat';
import type { AgentState } from '../stores/sessionStore';
import type { EventStreamHandle } from '../api/sse';

const EMPTY_MESSAGES: ChatMessage[] = [];

const STATE_STYLES: Record<AgentState, { label: string; color: string; dot: string }> = {
  idle: { label: 'Idle', color: 'text-crust-text-muted/50', dot: 'bg-crust-text-muted/30' },
  thinking: { label: 'Thinking', color: 'text-crust-amber', dot: 'bg-crust-amber animate-pulse' },
  tool: { label: 'Running tool', color: 'text-crust-mint', dot: 'bg-crust-mint animate-pulse' },
  done: { label: 'Done', color: 'text-green-400', dot: 'bg-green-400' },
  error: { label: 'Error', color: 'text-crust-ember', dot: 'bg-crust-ember' },
};

const headerActionClass = 'inline-flex h-8 w-8 items-center justify-center rounded-md border border-crust-mint/20 text-crust-text-muted transition hover:border-crust-mint/50 hover:text-crust-mint disabled:cursor-not-allowed disabled:opacity-40';

export function ChatPanel() {
  const activeSessionId = useSessionStore((state) => state.activeSessionId);
  const messages = useSessionStore((state) => (state.activeSessionId ? state.messagesBySession[state.activeSessionId] ?? EMPTY_MESSAGES : EMPTY_MESSAGES));
  const setMessages = useSessionStore((state) => state.setMessages);
  const applyEvent = useSessionStore((state) => state.applyEvent);
  const agentState = useSessionStore((state) => state.agentState);
  const notify = useToastStore((state) => state.notify);
  const streamHandleRef = useRef<EventStreamHandle | null>(null);
  const [contextInfo, setContextInfo] = useState<ContextInfo | null>(null);

  const sessionQuery = useQuery({
    queryKey: ['session', activeSessionId],
    queryFn: () => crustApi.getSession(activeSessionId!),
    enabled: Boolean(activeSessionId),
  });

  useEffect(() => {
    if (sessionQuery.data) {
      setMessages(sessionQuery.data.session.id, sessionQuery.data.messages);
    }
  }, [sessionQuery.data, setMessages]);

  useEffect(() => {
    streamHandleRef.current?.close();
    streamHandleRef.current = null;

    if (!activeSessionId) {
      return;
    }

    const handle = crustApi.streamSession(
      activeSessionId,
      (event: ChatEvent) => applyEvent(event),
      (error: Error) => {
        notify(`Stream error: ${error.message}`, 'error');
      },
    );

    streamHandleRef.current = handle;

    return () => {
      handle.close();
      streamHandleRef.current = null;
    };
  }, [activeSessionId, applyEvent, notify]);

  const sendMutation = useMutation({
    mutationFn: (content: string) => crustApi.sendMessage(activeSessionId!, { content }),
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to send message', 'error'),
  });

  const clearMutation = useMutation({
    mutationFn: () => crustApi.clearSession(activeSessionId!),
    onSuccess: () => setMessages(activeSessionId!, []),
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to clear session', 'error'),
  });

  const compactMutation = useMutation({
    mutationFn: () => crustApi.compactSession(activeSessionId!),
    onSuccess: () => sessionQuery.refetch(),
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to compact session', 'error'),
  });

  const contextMutation = useMutation({
    mutationFn: () => crustApi.getContext(activeSessionId!),
    onSuccess: (data) => setContextInfo(data),
    onError: (error) => notify(error instanceof Error ? error.message : 'Failed to load context', 'error'),
  });

  if (!activeSessionId) {
    return <div className="grid min-h-0 flex-1 place-items-center text-crust-text-muted/60">No session selected.</div>;
  }

  const stateStyle = STATE_STYLES[agentState];
  const isRunning = agentState === 'thinking' || agentState === 'tool';
  const contextPercent = contextInfo ? Math.min(100, Math.max(0, contextInfo.estimated_ratio * 100)) : null;

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center justify-between border-b border-crust-mint/20 bg-crust-panel/60 px-4 py-2.5 sm:px-6">
        <div className="flex items-center gap-4">
          <div>
            <p className="text-[0.65rem] uppercase tracking-[0.24em] text-crust-mint">Crust Chat</p>
            <h2 className="mt-0.5 text-lg font-semibold text-crust-text">Chat Console</h2>
          </div>
          <div className="flex items-center gap-1">
            <button
              className={headerActionClass}
              disabled={isRunning || clearMutation.isPending}
              onClick={() => clearMutation.mutate()}
              title="Clear all messages"
              aria-label="Clear all messages"
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            </button>
            <button
              className={headerActionClass}
              disabled={isRunning || compactMutation.isPending}
              onClick={() => compactMutation.mutate()}
              title="Compact old messages into summary"
              aria-label="Compact old messages into summary"
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 14h16"/><path d="M4 10h16"/><path d="M8 6h8"/><path d="M8 18h8"/></svg>
            </button>
            <button
              className={`inline-flex h-8 w-8 items-center justify-center rounded-md border transition disabled:cursor-not-allowed disabled:opacity-40 ${
                contextInfo
                  ? 'border-crust-mint/70 bg-crust-mint/10 text-crust-mint shadow-inner shadow-crust-mint/10'
                  : 'border-crust-mint/20 text-crust-text-muted hover:border-crust-mint/50 hover:text-crust-mint'
              }`}
              disabled={contextMutation.isPending}
              onClick={() => {
                if (contextInfo) {
                  setContextInfo(null);
                } else {
                  contextMutation.mutate();
                }
              }}
              title="Toggle context status"
              aria-label="Toggle context status"
              aria-pressed={Boolean(contextInfo)}
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 8v4"/><path d="M12 16h.01"/></svg>
            </button>
          </div>
        </div>
        <div className="flex items-center gap-3">
          {contextPercent !== null && (
            <div className="hidden items-center gap-2 text-xs text-crust-text-muted sm:flex" title={`${contextInfo!.estimated_tokens.toLocaleString()} / ${contextInfo!.context_window.toLocaleString()} tokens`}>
              <span>Ctx</span>
              <span className="h-1.5 w-20 overflow-hidden rounded-full bg-crust-panel">
                <span className="block h-full rounded-full bg-crust-mint" style={{ width: `${contextPercent}%` }} />
              </span>
              <span className="tabular-nums">{contextPercent.toFixed(0)}%</span>
            </div>
          )}
          <div className={`flex items-center gap-2 text-sm ${stateStyle.color}`}>
            <span className={`h-2 w-2 rounded-full ${stateStyle.dot}`} />
            <span className="font-medium">{stateStyle.label}</span>
          </div>
        </div>
      </div>

      {contextInfo && (
        <div className="shrink-0 border-b border-crust-mint/15 bg-crust-coal/60 px-4 py-2 sm:px-6">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-1 text-xs text-crust-text-muted">
            <span><span className="text-crust-text-subtle/60">Model:</span> {contextInfo.model}</span>
            <span><span className="text-crust-text-subtle/60">Context:</span> {contextInfo.estimated_tokens.toLocaleString()} / {contextInfo.context_window.toLocaleString()} toks ({(contextInfo.estimated_ratio * 100).toFixed(1)}%)</span>
            <span><span className="text-crust-text-subtle/60">Messages:</span> {contextInfo.context_messages} ({contextInfo.total_messages} total)</span>
            {contextInfo.has_summary && <span className="text-crust-amber">summary active</span>}
            {contextInfo.compacted_until > 0 && <span className="text-crust-text-subtle/60">compacted: {contextInfo.compacted_until}</span>}
          </div>
        </div>
      )}

      <MessageList loading={sessionQuery.isLoading} messages={messages} />
      <Composer disabled={sendMutation.isPending || isRunning} onSend={(content) => sendMutation.mutate(content)} />
    </section>
  );
}

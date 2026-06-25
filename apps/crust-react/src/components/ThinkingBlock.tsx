import type { ThinkingState } from '../types/chat';

export function ThinkingBlock({ thinking }: { thinking: ThinkingState }) {
  return (
    <div className="mb-3 rounded-xl border border-crust-amber/30 bg-crust-amber/10 p-3 text-sm text-amber-100">
      <div className="mb-1 flex items-center justify-between gap-3 text-xs uppercase tracking-[0.18em] text-crust-amber">
        <span>Thinking</span>
        <time>{new Date(thinking.startedAt).toLocaleTimeString()}</time>
      </div>
      <p>{thinking.summary}</p>
    </div>
  );
}

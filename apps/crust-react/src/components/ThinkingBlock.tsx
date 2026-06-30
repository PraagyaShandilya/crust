import type { ThinkingState } from '../types/chat';

export function ThinkingBlock({ thinking }: { thinking: ThinkingState }) {
  if (!thinking.summary) return null;

  return (
    <div className="rounded-lg border border-crust-amber/30 bg-crust-amber/5 px-3 py-1.5">
      <div className="mb-0.5 flex items-center gap-2 text-[0.6rem] uppercase tracking-[0.18em] text-crust-amber/70">
        <span>Thinking</span>
      </div>
      <p className="whitespace-pre-wrap font-mono text-[0.72rem] leading-4 text-crust-text-muted/60">
        {thinking.summary}
      </p>
    </div>
  );
}

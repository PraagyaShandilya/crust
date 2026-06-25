import type { ToolCall } from '../types/chat';

export function ToolCallCard({ toolCall }: { toolCall: ToolCall }) {
  return (
    <div className="mt-3 overflow-hidden rounded-xl border border-white/10 bg-black/20">
      <div className="flex items-center justify-between gap-3 border-b border-white/10 px-3 py-2">
        <span className="font-mono text-sm text-crust-mint">{toolCall.name}</span>
        <span className="rounded-full bg-white/10 px-2 py-0.5 text-xs text-slate-300">{toolCall.status}</span>
      </div>
      {toolCall.input !== undefined && <pre className="overflow-x-auto p-3 text-xs text-slate-300">{JSON.stringify(toolCall.input, null, 2)}</pre>}
      {toolCall.output && <p className="border-t border-white/10 px-3 py-2 text-sm text-slate-300">{toolCall.output}</p>}
      {toolCall.error && <p className="border-t border-red-400/20 px-3 py-2 text-sm text-red-200">{toolCall.error}</p>}
    </div>
  );
}

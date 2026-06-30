import { useState } from 'react';
import type { ToolCall } from '../types/chat';

const TOOL_STYLES: Record<string, { label: string; className: string }> = {
  shell_calling_tool: {
    label: 'Shell',
    className: 'border-orange-400/50 bg-orange-950/25 text-orange-100',
  },
  read_file_tool: {
    label: 'Read File',
    className: 'border-amber-300/50 bg-amber-950/20 text-amber-100',
  },
  write_file_tool: {
    label: 'Write File',
    className: 'border-red-400/50 bg-red-950/20 text-red-100',
  },
  edit_file_tool: {
    label: 'Edit File',
    className: 'border-yellow-400/50 bg-yellow-950/20 text-yellow-100',
  },
  web_search_tool: {
    label: 'Web Search',
    className: 'border-fuchsia-400/50 bg-fuchsia-950/20 text-fuchsia-100',
  },
  langgraph_run_tool: {
    label: 'LangGraph',
    className: 'border-sky-400/50 bg-sky-950/20 text-sky-100',
  },
};

function styleForTool(name: string) {
  return TOOL_STYLES[name] ?? {
    label: name.replace(/_/g, ' '),
    className: 'border-crust-mint/40 bg-crust-mint/10 text-crust-text-muted',
  };
}

function truncate(text: string, max: number) {
  if (text.length <= max) return text;
  return text.slice(0, max) + '…';
}

export function ToolCallCard({ toolCall }: { toolCall: ToolCall }) {
  const style = styleForTool(toolCall.name);
  const [expanded, setExpanded] = useState(false);

  const inputText = toolCall.input !== undefined ? JSON.stringify(toolCall.input, null, 2) : '';
  const outputText = toolCall.output ?? '';

  return (
    <div className={`overflow-hidden rounded-lg border text-xs ${style.className}`}>
      <button
        className="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left"
        onClick={() => setExpanded((v) => !v)}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="font-mono font-semibold">{style.label}</span>
          <span className="truncate opacity-50">{truncate(inputText, 60)}</span>
        </div>
        <span className="shrink-0 rounded-full bg-crust-panel/40 px-1.5 py-0.5 text-[0.6rem] uppercase tracking-wider">
          {toolCall.status}
        </span>
      </button>

      {expanded && (
        <div className="border-t border-current/20">
          {toolCall.input !== undefined && (
            <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-words bg-crust-panel/30 p-2 font-mono text-[0.7rem] opacity-80">
              {inputText}
            </pre>
          )}
          {outputText && (
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words border-t border-current/15 bg-crust-panel/20 p-2 font-mono text-[0.7rem] opacity-70">
              {outputText}
            </pre>
          )}
          {toolCall.error && (
            <p className="border-t border-red-400/30 px-2 py-1.5 text-red-200">{toolCall.error}</p>
          )}
        </div>
      )}

      {!expanded && outputText && (
        <div className="border-t border-current/15 px-3 py-1 font-mono text-[0.65rem] opacity-50">
          {truncate(outputText, 80)}
        </div>
      )}
    </div>
  );
}

export type SseHandler<T> = (event: T) => void;

export interface EventStreamHandle {
  close: () => void;
}

export function openJsonEventStream<T>(url: string, onEvent: SseHandler<T>, onError?: (error: Error) => void): EventStreamHandle {
  const source = new EventSource(url);

  const handleMessage = (message: MessageEvent<string>) => {
    try {
      onEvent(JSON.parse(message.data) as T);
    } catch (error) {
      onError?.(error instanceof Error ? error : new Error('Failed to parse SSE event'));
    }
  };

  source.onmessage = handleMessage;
  source.addEventListener('agent_event', handleMessage);

  source.onerror = () => {
    onError?.(new Error(`Event stream failed: ${url}`));
    source.close();
  };

  return {
    close: () => source.close(),
  };
}

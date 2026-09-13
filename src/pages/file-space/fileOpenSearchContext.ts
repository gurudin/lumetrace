export interface FileOpenSearchContext {
  fileId: string;
  query: string;
  lineNumber: number | null;
  pageNumber: number | null;
  matchIndex: number;
}

const searchContextByOpenEvent = new WeakMap<Event, FileOpenSearchContext>();

export function attachFileOpenSearchContext<T extends Event>(
  event: T,
  context?: FileOpenSearchContext | null,
) {
  if (context?.query.trim()) searchContextByOpenEvent.set(event, context);
  return event;
}

export function createFileOpenMouseEvent(context?: FileOpenSearchContext | null) {
  return attachFileOpenSearchContext(new MouseEvent("dblclick", {
    bubbles: true,
    cancelable: true,
    button: 0,
    view: window,
  }), context);
}

export function fileOpenSearchContextFromEvent(event: Event, fileId: string) {
  const context = searchContextByOpenEvent.get(event);
  return context?.fileId === fileId ? context : null;
}

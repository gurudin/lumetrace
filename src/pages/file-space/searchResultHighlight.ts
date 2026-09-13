import type { RefObject } from "react";
import { useLayoutEffect } from "react";
import type { FileOpenSearchContext } from "./fileOpenSearchContext";
import { splitSearchText } from "./searchTextHighlight";

const highlightSelector = "mark[data-file-search-highlight]";

interface SourcePositionElement extends HTMLElement {
  dataset: DOMStringMap & {
    sourceStartLine?: string;
    sourceEndLine?: string;
  };
}

function sourceRange(mark: HTMLElement) {
  const positioned = mark.closest<SourcePositionElement>("[data-source-start-line]");
  const start = Number(positioned?.dataset.sourceStartLine);
  const end = Number(positioned?.dataset.sourceEndLine);
  return Number.isFinite(start) && start > 0
    ? { start, end: Number.isFinite(end) && end >= start ? end : start }
    : null;
}

function removeSearchHighlights(root: HTMLElement) {
  root.querySelectorAll<HTMLElement>(highlightSelector).forEach((mark) => {
    mark.replaceWith(document.createTextNode(mark.textContent ?? ""));
  });
  root.normalize();
}

function targetHighlight(marks: HTMLElement[], context: FileOpenSearchContext) {
  if (context.lineNumber) {
    const exact = marks.find((mark) => {
      const range = sourceRange(mark);
      return range && context.lineNumber! >= range.start && context.lineNumber! <= range.end;
    });
    if (exact) return exact;

    const textLine = marks.find((mark) => Number(mark.dataset.searchLine) === context.lineNumber);
    if (textLine) return textLine;
  }
  return marks[Math.min(context.matchIndex, Math.max(0, marks.length - 1))] ?? null;
}

export function highlightSearchResult(root: HTMLElement, context: FileOpenSearchContext) {
  removeSearchHighlights(root);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const textNodes: Text[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!(node instanceof Text) || !node.data) continue;
    const parent = node.parentElement;
    if (!parent || parent.closest("textarea, script, style, mark[data-file-search-highlight]")) continue;
    textNodes.push(node);
  }

  let renderedLine = 1;
  textNodes.forEach((textNode) => {
    const parts = splitSearchText(textNode.data, context.query);
    if (!parts.some((part) => part.highlighted)) {
      renderedLine += (textNode.data.match(/\n/g) ?? []).length;
      return;
    }
    const fragment = document.createDocumentFragment();
    parts.forEach((part) => {
      if (part.highlighted) {
        const mark = document.createElement("mark");
        mark.dataset.fileSearchHighlight = "true";
        mark.dataset.searchLine = String(renderedLine);
        mark.className = "file-preview-search-highlight";
        mark.textContent = part.text;
        fragment.append(mark);
      } else {
        fragment.append(document.createTextNode(part.text));
      }
      renderedLine += (part.text.match(/\n/g) ?? []).length;
    });
    textNode.replaceWith(fragment);
  });

  const marks = Array.from(root.querySelectorAll<HTMLElement>(highlightSelector));
  const target = targetHighlight(marks, context);
  if (target) {
    target.classList.add("is-current");
    window.requestAnimationFrame(() => {
      if (target.isConnected) target.scrollIntoView({ block: "center", inline: "nearest" });
    });
  }
  return () => removeSearchHighlights(root);
}

export function useSearchResultHighlight(
  rootRef: RefObject<HTMLElement | null>,
  context: FileOpenSearchContext | null,
  contentKey: string,
) {
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || !context?.query.trim()) return undefined;
    return highlightSearchResult(root, context);
  }, [contentKey, context, rootRef]);
}

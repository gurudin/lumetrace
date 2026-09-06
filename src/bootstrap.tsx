import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./shared/i18n/i18n";
import { shouldPreserveNativeContextMenuForTarget } from "./shared/ui/nativeContextMenu";
import "./styles.css";

/** Shared application entry. Editions must not fork the file workspace UI. */
export function mountLumeTrace(element: HTMLElement) {
  const onContextMenu = (event: MouseEvent) => {
    if (!shouldPreserveNativeContextMenuForTarget(event.target)) event.preventDefault();
  };
  document.addEventListener("contextmenu", onContextMenu, { capture: true });
  const root = createRoot(element);
  root.render(<StrictMode><App /></StrictMode>);
  return () => {
    root.unmount();
    document.removeEventListener("contextmenu", onContextMenu, { capture: true });
  };
}

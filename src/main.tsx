import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./shared/i18n/i18n";
import { shouldPreserveNativeContextMenuForTarget } from "./shared/ui/nativeContextMenu";
import "./styles.css";

document.addEventListener("contextmenu", (event) => {
  if (!shouldPreserveNativeContextMenuForTarget(event.target)) event.preventDefault();
}, { capture: true });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

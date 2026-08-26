import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./shared/i18n/i18n";
import "./styles.css";

document.addEventListener("contextmenu", (event) => event.preventDefault(), { capture: true });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

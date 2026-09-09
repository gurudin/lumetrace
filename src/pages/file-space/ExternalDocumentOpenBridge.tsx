import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { isTauri } from "@tauri-apps/api/core";
import { AlertTriangle, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { resolveFileDoubleClickRoute } from "./fileOpenRouting";
import "./external-document-open-bridge.css";

interface ExternalDocumentRequest {
  fileId: string;
  name: string;
  action: "open" | "reveal";
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function openFailure(error: unknown) {
  const raw = errorText(error);
  const marker = "DEFAULT_APPLICATION_UNAVAILABLE:";
  return raw.startsWith(marker)
    ? { message: raw.slice(marker.length), canChooseApplication: true }
    : { message: raw, canChooseApplication: false };
}

function externalDocumentFromTarget(target: EventTarget | null) {
  if (!(target instanceof Element)) return null;
  const button = target.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
  const card = button?.closest<HTMLElement>(".file-space-file-card");
  const fileId = button?.dataset.fileId ?? card?.dataset.fileId;
  const name = button?.title ?? "";
  if (!button || !fileId || !name.trim()) return null;
  const route = resolveFileDoubleClickRoute(
    name,
    Boolean(button.querySelector(".file-space-file-art.has-preview")),
  );
  if (route === "internal-preview") return null;
  return {
    button,
    request: {
      fileId,
      name,
      action: route === "external-open" ? "open" : "reveal",
    } satisfies ExternalDocumentRequest,
  };
}

export function ExternalDocumentOpenBridge() {
  const invoke = useWorkspaceInvoke();
  const { t } = useTranslation();
  const copy = {
    opening: (name: string) => t("fileSpace.externalDocument.opening", { name }),
    revealing: (name: string) => t("fileSpace.externalDocument.revealing", { name }),
    failed: (name: string) => t("fileSpace.externalDocument.failed", { name }),
    choose: t("fileSpace.externalDocument.choose"),
    choosing: t("fileSpace.externalDocument.choosing"),
    dismiss: t("fileSpace.externalDocument.dismiss"),
    desktopOnly: t("fileSpace.externalDocument.desktopOnly"),
  };
  const [request, setRequest] = useState<ExternalDocumentRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [canChooseApplication, setCanChooseApplication] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const clearRequestTimerRef = useRef<number | null>(null);
  const operationRef = useRef(0);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const presence = usePresence(open);

  const closeNotice = useCallback(() => {
    setOpen(false);
    if (clearRequestTimerRef.current !== null) window.clearTimeout(clearRequestTimerRef.current);
    clearRequestTimerRef.current = window.setTimeout(() => {
      clearRequestTimerRef.current = null;
      setRequest(null);
    }, 220);
  }, []);

  const dismiss = useCallback(() => {
    operationRef.current += 1;
    setChoosing(false);
    setError(null);
    setCanChooseApplication(false);
    closeNotice();
    window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, [closeNotice]);

  const openDocument = useCallback(async (target: ExternalDocumentRequest) => {
    const operation = operationRef.current + 1;
    operationRef.current = operation;
    if (clearRequestTimerRef.current !== null) {
      window.clearTimeout(clearRequestTimerRef.current);
      clearRequestTimerRef.current = null;
    }
    setRequest(target);
    setOpen(true);
    setError(null);
    setCanChooseApplication(false);
    setChoosing(false);
    try {
      if (!isTauri()) throw new Error(copy.desktopOnly);
      await invoke(target.action === "open" ? "open_file_space_file" : "reveal_file_space_file", { fileId: target.fileId });
      if (operation !== operationRef.current) return;
      closeNotice();
    } catch (openError) {
      if (operation !== operationRef.current) return;
      const failure = openFailure(openError);
      setError(failure.message);
      setCanChooseApplication(target.action === "open" && failure.canChooseApplication);
    }
  }, [closeNotice, copy.desktopOnly]);

  const chooseApplication = useCallback(async () => {
    if (!request || choosing) return;
    const previousError = error;
    const operation = operationRef.current + 1;
    operationRef.current = operation;
    setError(null);
    setChoosing(true);
    try {
      const opened = await invoke<boolean>("choose_application_for_file_space_file", { fileId: request.fileId });
      if (operation !== operationRef.current) return;
      setChoosing(false);
      if (opened) closeNotice();
      else {
        setError(previousError ?? copy.failed(request.name));
        setCanChooseApplication(true);
      }
    } catch (chooseError) {
      if (operation !== operationRef.current) return;
      setChoosing(false);
      setError(errorText(chooseError));
      setCanChooseApplication(true);
    }
  }, [choosing, closeNotice, copy, error, request]);

  useEffect(() => {
    const handleDoubleClick = (event: MouseEvent) => {
      const card = externalDocumentFromTarget(event.target);
      if (!card || event.button !== 0) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      returnFocusRef.current = card.button;
      void openDocument(card.request);
    };

    document.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      document.removeEventListener("dblclick", handleDoubleClick, true);
      if (clearRequestTimerRef.current !== null) window.clearTimeout(clearRequestTimerRef.current);
      operationRef.current += 1;
    };
  }, [openDocument]);

  useEffect(() => {
    if (!request) return undefined;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      dismiss();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [dismiss, request]);

  if (!presence.mounted || !request) return null;

  return (
    <section
      className={`external-document-notice${error ? " is-error" : ""}`}
      data-state={presence.state}
      role={error ? "alert" : "status"}
      aria-live={error ? "assertive" : "polite"}
    >
      <span className="external-document-notice-icon" aria-hidden="true">
        {error ? <AlertTriangle size={17} /> : <LoaderCircle className="is-spinning" size={17} />}
      </span>
      <span className="external-document-notice-copy">
        <strong>{error
          ? copy.failed(request.name)
          : choosing
            ? copy.choosing
            : request.action === "open"
              ? copy.opening(request.name)
              : copy.revealing(request.name)}</strong>
        {error ? <small>{error}</small> : null}
      </span>
      {error && canChooseApplication ? (
        <button type="button" className="external-document-choose" onClick={() => void chooseApplication()}>
          {copy.choose}
        </button>
      ) : null}
      <button type="button" className="external-document-dismiss" onClick={dismiss} title={copy.dismiss} aria-label={copy.dismiss}>
        <X size={16} />
      </button>
    </section>
  );
}

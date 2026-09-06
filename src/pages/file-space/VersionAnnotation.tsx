import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle, Pencil, Star } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { applyVersionAnnotation, versionAnnotationUpdatedEvent, versionNoteLength, versionNoteLimit, type AnnotatedTimeline, type AnnotatedVersion, type VersionAnnotationUpdate } from "./versionAnnotationState";
import "./version-annotation.css";

export function useVersionAnnotationUpdates<T extends AnnotatedTimeline>(setTimeline: Dispatch<SetStateAction<T | null>>) {
  useEffect(() => {
    const update = (event: Event) => {
      const detail = (event as CustomEvent<VersionAnnotationUpdate>).detail;
      if (detail) setTimeline((timeline) => applyVersionAnnotation(timeline, detail));
    };
    window.addEventListener(versionAnnotationUpdatedEvent, update);
    return () => window.removeEventListener(versionAnnotationUpdatedEvent, update);
  }, [setTimeline]);
}

interface Props {
  workspaceId?: string;
  fileId: string;
  version: AnnotatedVersion & { versionNumber: number };
  disabled?: boolean;
}

/** Metadata controls are siblings of version selection, never nested buttons. */
export function VersionAnnotation({ workspaceId, fileId, version, disabled = false }: Props) {
  const { t } = useTranslation();
  const labelId = useId();
  const helpId = useId();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const editRef = useRef<HTMLButtonElement>(null);
  const busyRef = useRef(false);
  const restoreFocusPending = useRef(false);
  const mounted = useRef(true);
  const key = "fileSpace.versionAnnotation.";
  const unavailable = disabled || !workspaceId || busy;
  const noteLength = versionNoteLength(draft);
  const noteTooLong = noteLength > versionNoteLimit;

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (!editing) return;
    dialogRef.current?.showModal();
    fieldRef.current?.focus();
  }, [editing]);

  useLayoutEffect(() => {
    // Restore only after React has removed the native modal and re-enabled
    // the trigger. Focusing before that commit can leave focus on the body.
    if (!editing && !busy && restoreFocusPending.current) {
      restoreFocusPending.current = false;
      editRef.current?.focus({ preventScroll: true });
    }
  }, [busy, editing]);

  const closeEditor = () => {
    if (busyRef.current) return;
    restoreFocusPending.current = true;
    setEditing(false);
    setError(null);
  };
  const save = async (patch: { note?: string; isMilestone?: boolean }) => {
    if (!workspaceId || disabled || busyRef.current) return false;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const update = await invoke<VersionAnnotationUpdate>("update_file_version_annotation", {
        request: { workspaceId, fileId, versionId: version.id, ...patch },
      });
      window.dispatchEvent(new CustomEvent(versionAnnotationUpdatedEvent, { detail: update }));
      return true;
    } catch (failure) {
      if (mounted.current) setError(failure instanceof Error ? failure.message : String(failure));
      return false;
    } finally {
      busyRef.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const saveNote = async () => {
    if (noteTooLong) return;
    if (await save({ note: draft })) {
      if (mounted.current) closeEditor();
    }
  };
  const starLabel = t(key + (version.isMilestone ? "unstar" : "star"), { version: version.versionNumber });
  const editLabel = t(key + (version.note ? "edit" : "add"), { version: version.versionNumber });

  return <div className="file-version-annotation" onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
    <button type="button" className="file-version-star" disabled={unavailable} aria-label={starLabel} title={starLabel}
      aria-pressed={version.isMilestone ?? false} onClick={() => void save({ isMilestone: !version.isMilestone })}>
      {busy && !editing ? <LoaderCircle size={15} className="is-spinning" /> : <Star size={15} fill={version.isMilestone ? "currentColor" : "none"} />}
    </button>
    <button ref={editRef} type="button" className={`file-version-note${version.note ? " has-note" : ""}`} disabled={unavailable}
      aria-label={editLabel} title={version.note || editLabel} onClick={() => { setDraft(version.note ?? ""); setError(null); setEditing(true); }}>
      <Pencil size={12} aria-hidden="true" /><span className="file-version-note-text">{version.note || t(key + "addShort")}</span>
    </button>
    {error && !editing ? <p className="file-version-annotation-error" role="alert">{t(key + "saveError")} {error}</p> : null}
    {editing ? createPortal(
      <dialog ref={dialogRef} className="file-version-note-dialog" tabIndex={-1} aria-modal="true" aria-labelledby={labelId} aria-describedby={helpId}
        onCancel={(event) => { event.preventDefault(); closeEditor(); }}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Tab") {
            const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('textarea:not(:disabled), button:not(:disabled)'));
            const first = controls[0] ?? event.currentTarget;
            const last = controls.at(-1) ?? event.currentTarget;
            if (event.shiftKey ? document.activeElement === first : document.activeElement === last) {
              event.preventDefault(); (event.shiftKey ? last : first).focus();
            }
          }
          if (event.key === "Escape") { event.preventDefault(); closeEditor(); }
          if ((event.metaKey || event.ctrlKey) && (event.key === "Enter" || event.key.toLowerCase() === "s")) {
            event.preventDefault(); if (!event.nativeEvent.isComposing) void saveNote();
          }
        }}>
        <form onSubmit={(event) => { event.preventDefault(); void saveNote(); }}>
          <h2 id={labelId}>{t(key + "title", { version: version.versionNumber })}</h2>
          <p id={helpId}>{t(key + "hint")}</p>
          <textarea ref={fieldRef} value={draft} onChange={(event) => setDraft(event.target.value)} rows={3}
            aria-invalid={noteTooLong} aria-describedby={`${labelId}-count`}
            disabled={busy} aria-label={t(key + "field")} placeholder={t(key + "placeholder")} data-native-context-menu="true" />
          <small id={`${labelId}-count`} className={`file-version-note-count${noteTooLong ? " is-over-limit" : ""}`}>
            {noteLength} / {versionNoteLimit}{noteTooLong ? ` · ${t(key + "tooLong", { count: versionNoteLimit })}` : ""}
          </small>
          {error ? <p className="file-version-annotation-error" role="alert">{t(key + "saveError")} {error}</p> : null}
          <footer>
            <button type="button" disabled={busy} onClick={closeEditor}>{t(key + "cancel")}</button>
            <button type="submit" className="is-primary" disabled={busy || noteTooLong}>{busy ? t(key + "saving") : t(key + "save")}</button>
          </footer>
        </form>
      </dialog>, document.body) : null}
  </div>;
}

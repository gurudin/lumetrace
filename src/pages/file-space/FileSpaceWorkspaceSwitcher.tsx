import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronsUpDown, Folder } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWorkspaceExtension } from "../../shared/extensions/ApplicationExtension";
import { FileSpaceWorkspaceStatus, type FileSpaceWorkspaceDirectory, type FileSpaceWorkspaceMutation } from "./FileSpaceWorkspaceMenu";

export function FileSpaceWorkspaceSwitcher<T>({ directory, disabled = false, onWorkspaceChanged }: {
  directory: FileSpaceWorkspaceDirectory | null; disabled?: boolean; onWorkspaceChanged: (mutation: FileSpaceWorkspaceMutation<T>) => void;
}) {
  const { t } = useTranslation();
  const external = useWorkspaceExtension(), ExtraList = external?.List;
  const [open, setOpen] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const host = useRef<HTMLDivElement>(null), trigger = useRef<HTMLButtonElement>(null), menu = useRef<HTMLDivElement>(null);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => menu.current?.querySelector<HTMLButtonElement>('[role="menuitemradio"]:not(:disabled)')?.focus());
    const outside = (event: PointerEvent) => { if (!host.current?.contains(event.target as Node)) setOpen(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopPropagation(); setOpen(false); trigger.current?.focus(); } };
    window.addEventListener("pointerdown", outside); window.addEventListener("keydown", escape, true);
    return () => { cancelAnimationFrame(frame); window.removeEventListener("pointerdown", outside); window.removeEventListener("keydown", escape, true); };
  }, [open]);
  const selectLocal = async (id: string) => {
    if (busy || disabled) return;
    if (id === directory?.currentWorkspaceId) { close(); external?.onLocalSelect(id); return; }
    setBusy(true); setError("");
    try {
      const mutation = await invoke<FileSpaceWorkspaceMutation<T>>("switch_file_space_workspace", { workspaceId: id });
      onWorkspaceChanged(mutation); close(); external?.onLocalSelect(id);
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { setBusy(false); }
  };
  return <div className="file-space-workspace-switcher" ref={host}>
    <button ref={trigger} type="button" className="file-space-workspace-switcher-trigger" disabled={disabled || busy} aria-haspopup="menu" aria-expanded={open}
      aria-label={t("fileSpace.workspaces.switcher")} onClick={() => setOpen(value => !value)}>
      <FileSpaceWorkspaceStatus directory={directory} /><ChevronsUpDown size={13} aria-hidden="true" />
    </button>
    {open ? <div ref={menu} className="file-space-workspace-switcher-popover" role="menu" aria-label={t("fileSpace.workspaces.switcher")} onKeyDown={event => {
      if (event.key === "Tab") { setOpen(false); return; }
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const buttons = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]:not(:disabled)') ?? []);
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (index + (event.key === "ArrowUp" ? -1 : 1) + buttons.length) % buttons.length]?.focus();
    }}>
      <div role="group" aria-label={t("fileSpace.workspaces.localSection")}><p>{t("fileSpace.workspaces.localSection")}</p>
        {directory?.workspaces.filter(workspace => workspace.kind === "local" && workspace.rootPath).map(workspace => {
          const current = workspace.id === directory.currentWorkspaceId && !external?.active;
          return <button key={workspace.id} type="button" role="menuitemradio" aria-checked={current} disabled={disabled || busy} onClick={() => void selectLocal(workspace.id)}>
            <Folder size={15} /><span title={workspace.name}>{workspace.name}</span>{current ? <Check size={14} /> : null}
          </button>;
        })}
      </div>
      {ExtraList ? <ExtraList compact disabled={disabled || busy} onDone={close} /> : null}
      {error ? <p role="alert" className="file-space-workspace-error">{error}</p> : null}
    </div> : null}
  </div>;
}

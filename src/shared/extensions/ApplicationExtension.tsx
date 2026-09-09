import { createContext, useContext, useRef, useState, type ComponentType, type PropsWithChildren, type ReactNode } from "react";
import "./application-extension.css";

export interface ApplicationExtension {
  Entry: ComponentType<{ placement: "setup" | "settings"; onOpen: () => void; disabled: boolean }>;
  Page: ComponentType<{ active: boolean; onClose: () => void }>;
  initiallyOpen?: boolean;
  onVisibilityChange?: (visible: boolean) => void;
  WorkspaceProvider?: ComponentType<PropsWithChildren>;
}

/** Edition-owned storage surfaces reuse the shared workspace settings, never the local file commands. */
export interface WorkspaceExtension {
  active: boolean;
  name: string;
  typeLabel: string;
  onLocalSelect: () => void;
  List: ComponentType<{ disabled: boolean; onDone: () => void }>;
  surface: ReactNode;
  footerTarget: HTMLElement | null;
}
export const WorkspaceExtensionContext = createContext<WorkspaceExtension | null>(null);
export function useWorkspaceExtension() { return useContext(WorkspaceExtensionContext); }

const ExtensionContext = createContext<{
  extension: ApplicationExtension;
  active: boolean;
  pageActive: boolean;
  open: (returnFocus?: HTMLElement | null) => void;
} | null>(null);

export function useApplicationExtension() { return useContext(ExtensionContext); }

/** Optional edition-owned page. Shared workspace state and background jobs stay mounted. */
export function ApplicationExtensionHost({ extension, children }: PropsWithChildren<{ extension?: ApplicationExtension }>) {
  const Provider = extension?.WorkspaceProvider;
  return Provider ? <Provider><ExtensionHost extension={extension}>{children}</ExtensionHost></Provider>
    : <ExtensionHost extension={extension}>{children}</ExtensionHost>;
}

function ExtensionHost({ extension, children }: PropsWithChildren<{ extension?: ApplicationExtension }>) {
  const [active, setActive] = useState(extension?.initiallyOpen ?? false);
  const workspace = useWorkspaceExtension();
  const returnFocus = useRef<HTMLElement | null>(null);
  if (!extension) return children;
  const open = (target?: HTMLElement | null) => {
    returnFocus.current = target ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setActive(true);
    extension.onVisibilityChange?.(true);
  };
  const close = () => {
    setActive(false);
    extension.onVisibilityChange?.(false);
    window.requestAnimationFrame(() => {
      if (returnFocus.current?.isConnected) returnFocus.current.focus();
    });
  };
  const Page = extension.Page;
  const workspaceHidden = active || Boolean(workspace?.active);
  return <ExtensionContext.Provider value={{ extension, active: workspaceHidden, pageActive: active, open }}>
    <div className="application-workspace-surface" hidden={workspaceHidden} inert={workspaceHidden} aria-hidden={workspaceHidden}>{children}</div>
    {workspace ? <div className="application-extension-surface" hidden={active || !workspace.active} inert={active || !workspace.active} aria-hidden={active || !workspace.active}>
      {workspace.surface}
    </div> : null}
    <div className="application-extension-surface" hidden={!active} inert={!active} aria-hidden={!active}>
      <Page active={active} onClose={close} />
    </div>
  </ExtensionContext.Provider>;
}

export function ApplicationExtensionEntry({ placement, disabled = false, beforeOpen, returnFocus }: {
  placement: "setup" | "settings"; disabled?: boolean; beforeOpen?: () => void; returnFocus?: () => HTMLElement | null;
}) {
  const context = useApplicationExtension();
  if (!context) return null;
  const Entry = context.extension.Entry;
  return <Entry placement={placement} disabled={disabled} onOpen={() => {
    if (disabled) return;
    const target = returnFocus?.();
    beforeOpen?.();
    context.open(target);
  }} />;
}

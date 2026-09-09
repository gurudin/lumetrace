import { useLayoutEffect, useMemo, useRef } from "react";
import { useWorkspaceExtension } from "./ApplicationExtension";
import { bindWorkspaceInvoke } from "./workspaceCommands";

/** Delayed dialogs and callbacks retain their original source, including after unmount. */
export function useWorkspaceInvoke() {
  const source = useWorkspaceExtension()?.source ?? null;
  const alive = useRef(true);
  useLayoutEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  return useMemo(() => bindWorkspaceInvoke(source, () => alive.current), [source]);
}

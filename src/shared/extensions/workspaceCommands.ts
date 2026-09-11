import { invoke as nativeInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import type { ImagePreviewStore } from "./imagePreviewStore";

export interface WorkspaceCommandSource {
  key: string;
  /** Stable identity for version metadata; external spaces never borrow a local registry ID. */
  workspaceId?: string;
  /** Edition-owned, authenticated image transport; independent of version history. */
  imagePreviewUrl?: (fileId: string, updatedAt: number, purpose?: "thumbnail" | "detail") => string | null;
  /** Present only when the displayed preview is processed and an original can be fetched on demand. */
  imageOriginalUrl?: (fileId: string, updatedAt: number) => string | null;
  /** Resolve a detail source on open using transport metadata, without downloading image bodies. */
  resolveImagePreview?: (fileId: string, updatedAt: number) => Promise<{ source: string; originalSource: string | null } | null>;
  /** Optional workspace-owned loaded thumbnails, retained across card virtualization. */
  imagePreviews?: ImagePreviewStore;
  /** Original pixel dimensions, never the dimensions of a resized preview. */
  imageOriginalDimensions?: (fileId: string, updatedAt: number) => Promise<{ width: number; height: number } | null>;
  capabilities?: { write?: boolean; content?: boolean; history?: boolean; search?: boolean; ai?: boolean; backup?: boolean };
  invoke: <T>(command: string, args?: InvokeArgs) => Promise<T>;
}
let source: WorkspaceCommandSource | null = null;
// Whole-space archives are separate from individual file-version recovery.
const backupCommands = new Set([
  "export_file_space_backup", "inspect_file_space_backup", "restore_file_space_backup",
]);
// These manage the application/local registry, not the selected space's files.
const applicationCommands = new Set([
  "get_file_space_workspaces", "create_file_space_workspace", "switch_file_space_workspace",
  "rename_file_space_workspace", "remove_file_space_workspace", "open_feedback_channel",
  "get_ai_service_settings", "check_cloud_ai_connection", "check_local_llm_connection",
  "save_cloud_ai_settings", "save_local_llm_settings", "check_agent_clis", "check_agent_cli_status",
  "check_agent_cli", "get_agent_cli_settings", "save_agent_cli_settings",
]);
export function setWorkspaceCommandSource(next: WorkspaceCommandSource | null) {
  source = next;
  return () => { if (source === next) source = null; };
}
export async function workspaceInvoke<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  const captured = source;
  return bindWorkspaceInvoke(captured)(command, args, options);
}
export function bindWorkspaceInvoke(captured: WorkspaceCommandSource | null, mounted = () => true) {
  return async <T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> => {
  const current = () => mounted() && source === captured;
  if (!current()) throw new Error("The active workspace changed. Please try again.");
  if (captured?.capabilities?.backup === false && backupCommands.has(command)) {
    throw new Error("Whole-workspace backup and restore are disabled for this workspace.");
  }
  if (applicationCommands.has(command)) return nativeInvoke<T>(command, args, options);
  const result = captured ? await captured.invoke<T>(command, args) : await nativeInvoke<T>(command, args, options);
  if (!current()) throw new Error("The active workspace changed. Please try again.");
  return result;
  };
}

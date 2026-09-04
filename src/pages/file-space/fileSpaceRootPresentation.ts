export type FileSpaceRootStatus = "ready" | "unconfigured" | "missing" | "notDirectory" | "notWritable";
export type FileSpaceRootView = "loading" | "loadError" | "setup" | "workspace";

export function fileSpaceRootView(
  loading: boolean,
  loadError: string | null,
  rootStatus: FileSpaceRootStatus,
): FileSpaceRootView {
  if (loading) return "loading";
  if (loadError) return "loadError";
  return rootStatus === "ready" ? "workspace" : "setup";
}

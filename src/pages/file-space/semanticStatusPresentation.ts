export type SemanticStatusReadState = "idle" | "loading" | "ready" | "error";
export type SemanticStatusView = "loading" | "status" | "error";

export function semanticStatusView(readState: SemanticStatusReadState): SemanticStatusView {
  if (readState === "error") return "error";
  if (readState === "ready") return "status";
  return "loading";
}

export function shouldPollSemanticStatus(
  readState: SemanticStatusReadState,
  pipelineState: string,
  actionBusy = false,
) {
  return !actionBusy
    && readState === "ready"
    && ["downloading", "validating", "indexing"].includes(pipelineState);
}

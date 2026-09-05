import { diffTextVersions } from "./versionDiff";
import { previewDiffExceedsRenderBudget } from "./previewDiffData";

self.onmessage = (event: MessageEvent<{ before: Uint8Array; after: Uint8Array }>) => {
  try {
    const decoder = new TextDecoder("utf-8", { fatal: true });
    const result = diffTextVersions(decoder.decode(event.data.before), decoder.decode(event.data.after));
    self.postMessage(previewDiffExceedsRenderBudget(result)
      ? { status: "tooManyChanges", result: null }
      : { status: "ready", result });
  } catch (error) {
    self.postMessage({ status: "error", result: null, error: String(error) });
  }
};

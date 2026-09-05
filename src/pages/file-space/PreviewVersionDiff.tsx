import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, GitCompareArrows, LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FileVersionDiff, type VersionDiffStatus } from "./FileVersionDiff";
import { defaultVersionComparison, type VersionDiffResult } from "./versionDiff";
import { readPreviewDiffPair, type PreviewDiffVersion, type PreviewSnapshotReader } from "./previewDiffData";
import "./preview-version-diff.css";

export function PreviewDiffButton({ active, versionCount, disabled = false, disabledReason, onClick }: {
  active: boolean;
  versionCount: number;
  disabled?: boolean;
  disabledReason?: string;
  onClick: () => void;
}) {
  const { t } = useTranslation();
  const label = t("fileSpace.timeline.diff.title");
  return <button
    className={`file-preview-diff-button${active ? " is-active" : ""}`}
    type="button"
    aria-label={label}
    aria-pressed={active}
    disabled={disabled || versionCount < 2}
    title={versionCount < 2 ? t("fileSpace.timeline.diff.needTwoVersions") : disabledReason ?? label}
    onClick={onClick}
  ><GitCompareArrows size={15} aria-hidden="true" />Diff</button>;
}

interface PreviewVersionDiffProps {
  fileId: string;
  selectedVersionId: string | null;
  versions: PreviewDiffVersion[] | undefined;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
  readSnapshot?: PreviewSnapshotReader;
}

const readSnapshot: PreviewSnapshotReader = (fileId, versionId) => invoke<number[]>(
  "read_task_file_version", { fileId, versionId },
);

export function PreviewVersionDiff(props: PreviewVersionDiffProps) {
  const { t } = useTranslation();
  if (props.loading || props.error || !props.versions) {
    return <div className={`file-version-diff-state${props.error ? " is-error" : ""}`} role={props.error ? "alert" : "status"}>
      {props.error ? <AlertTriangle size={18} /> : <LoaderCircle className="is-spinning" size={18} />}
      <span>{props.error ? t("fileSpace.preview.common.timelineError") : t("fileSpace.timeline.diff.loading")}</span>
      {props.error ? <><span>{props.error}</span><button type="button" onClick={props.onRetry}>{t("fileSpace.timeline.diff.retry")}</button></> : null}
    </div>;
  }
  return <LoadedPreviewVersionDiff
    key={`${props.fileId}:${props.selectedVersionId}:${props.versions.map((version) => version.id).join(",")}`}
    fileId={props.fileId}
    versions={props.versions}
    selectedVersionId={props.selectedVersionId}
    readSnapshot={props.readSnapshot ?? readSnapshot}
  />;
}

function LoadedPreviewVersionDiff({ fileId, versions, selectedVersionId, readSnapshot }: {
  fileId: string;
  versions: PreviewDiffVersion[];
  selectedVersionId: string | null;
  readSnapshot: PreviewSnapshotReader;
}) {
  const [pair, setPair] = useState(() => defaultVersionComparison(versions, selectedVersionId ?? ""));
  const [retry, setRetry] = useState(0);
  const [response, setResponse] = useState<{
    status: VersionDiffStatus; result: VersionDiffResult | null; error?: string;
  }>({ status: "loading", result: null });

  useEffect(() => {
    const before = versions.find((version) => version.id === pair?.beforeVersionId);
    const after = versions.find((version) => version.id === pair?.afterVersionId);
    if (!before || !after) return;
    let disposed = false;
    let worker: Worker | undefined;
    setResponse({ status: "loading", result: null });
    void (async () => {
      try {
        const bytes = await readPreviewDiffPair(fileId, before, after, readSnapshot);
        if (disposed) return;
        if (!bytes) {
          setResponse({ status: "tooLarge", result: null });
          return;
        }
        worker = new Worker(new URL("./previewVersionDiff.worker.ts", import.meta.url), { type: "module" });
        worker.onmessage = (event: MessageEvent<typeof response>) => {
          if (!disposed) setResponse(event.data);
          worker?.terminate();
        };
        worker.onerror = (event) => {
          if (!disposed) setResponse({ status: "error", result: null, error: event.message });
          worker?.terminate();
        };
        worker.postMessage(bytes, [bytes.before.buffer, bytes.after.buffer]);
      } catch (error) {
        if (!disposed) setResponse({ status: "error", result: null, error: String(error) });
        worker?.terminate();
      }
    })();
    return () => { disposed = true; worker?.terminate(); };
  }, [fileId, versions, pair, retry, readSnapshot]);

  return <div className="file-preview-diff-body" data-native-context-menu="true">
    <FileVersionDiff
      versions={versions}
      beforeVersionId={pair?.beforeVersionId ?? null}
      afterVersionId={pair?.afterVersionId ?? null}
      status={response.status}
      result={response.result}
      error={response.error ?? null}
      onChangeBefore={(id) => { setResponse({ status: "loading", result: null }); setPair((current) => current && { ...current, beforeVersionId: id }); }}
      onChangeAfter={(id) => { setResponse({ status: "loading", result: null }); setPair((current) => current && { ...current, afterVersionId: id }); }}
      onSwap={() => { setResponse({ status: "loading", result: null }); setPair((current) => current && { beforeVersionId: current.afterVersionId, afterVersionId: current.beforeVersionId }); }}
      onRetry={() => setRetry((value) => value + 1)}
    />
  </div>;
}

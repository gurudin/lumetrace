import { AlertTriangle, ArrowLeftRight, Columns2, LoaderCircle, Rows3 } from "lucide-react";
import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { MacSelect, type MacSelectOption } from "../../shared/ui/MacSelect";
import type {
  VersionDiffContentRow,
  VersionDiffResult,
  VersionDiffSegment,
  VersionDiffSide,
} from "./versionDiff";
import "./file-version-diff.css";

export type VersionDiffStatus = "idle" | "loading" | "ready" | "unsupported" | "tooLarge" | "tooManyChanges" | "error";

interface DiffVersion {
  id: string;
  versionNumber: number;
  isCurrent: boolean;
}

interface FileVersionDiffProps {
  versions: DiffVersion[];
  beforeVersionId: string | null;
  afterVersionId: string | null;
  status: VersionDiffStatus;
  result: VersionDiffResult | null;
  error: string | null;
  onChangeBefore: (versionId: string) => void;
  onChangeAfter: (versionId: string) => void;
  onSwap: () => void;
  onRetry: () => void;
}

type DiffViewMode = "unified" | "split";

function DiffSegments({ segments }: { segments: VersionDiffSegment[] }) {
  return <>{segments.map((segment, index) => (
    <mark className={`is-${segment.kind}`} key={`${index}:${segment.text}`}>{segment.text}</mark>
  ))}</>;
}

function UnifiedLine({
  side,
  kind,
  beforeLineNumber,
  afterLineNumber,
}: {
  side: VersionDiffSide;
  kind: "context" | "added" | "removed";
  beforeLineNumber: number | null;
  afterLineNumber: number | null;
}) {
  return (
    <div className={`file-version-diff-unified-line is-${kind}`} role="row">
      <span aria-hidden="true">{beforeLineNumber ?? ""}</span>
      <span aria-hidden="true">{afterLineNumber ?? ""}</span>
      <b aria-hidden="true">{kind === "added" ? "+" : kind === "removed" ? "−" : ""}</b>
      <code><DiffSegments segments={side.segments} /></code>
    </div>
  );
}

function UnifiedRow({ row }: { row: VersionDiffContentRow }) {
  if (row.kind === "context" && row.after) {
    return (
      <UnifiedLine
        side={row.after}
        kind="context"
        beforeLineNumber={row.before?.lineNumber ?? null}
        afterLineNumber={row.after.lineNumber}
      />
    );
  }
  return <>
    {row.before ? (
      <UnifiedLine side={row.before} kind="removed" beforeLineNumber={row.before.lineNumber} afterLineNumber={null} />
    ) : null}
    {row.after ? (
      <UnifiedLine side={row.after} kind="added" beforeLineNumber={null} afterLineNumber={row.after.lineNumber} />
    ) : null}
  </>;
}

function SplitSide({ side, kind }: { side: VersionDiffSide | null; kind: "context" | "added" | "removed" }) {
  return (
    <div className={`file-version-diff-split-side is-${side ? kind : "empty"}`}>
      <span aria-hidden="true">{side?.lineNumber ?? ""}</span>
      <b aria-hidden="true">{side && kind === "added" ? "+" : side && kind === "removed" ? "−" : ""}</b>
      <code>{side ? <DiffSegments segments={side.segments} /> : null}</code>
    </div>
  );
}

function SplitRow({ row }: { row: VersionDiffContentRow }) {
  const context = row.kind === "context";
  return (
    <div className="file-version-diff-split-row" role="row">
      <SplitSide side={row.before} kind={context ? "context" : "removed"} />
      <SplitSide side={row.after} kind={context ? "context" : "added"} />
    </div>
  );
}

export function FileVersionDiff({
  versions,
  beforeVersionId,
  afterVersionId,
  status,
  result,
  error,
  onChangeBefore,
  onChangeAfter,
  onSwap,
  onRetry,
}: FileVersionDiffProps) {
  const { t } = useTranslation();
  const titleId = useId();
  const [viewMode, setViewMode] = useState<DiffViewMode>("unified");
  const beforeVersion = versions.find((version) => version.id === beforeVersionId) ?? null;
  const afterVersion = versions.find((version) => version.id === afterVersionId) ?? null;
  const options = useMemo<MacSelectOption<string>[]>(() => versions.map((version) => ({
    value: version.id,
    label: version.isCurrent
      ? t("fileSpace.timeline.diff.versionCurrent", { version: version.versionNumber })
      : t("fileSpace.timeline.diff.version", { version: version.versionNumber }),
  })), [t, versions]);
  const beforeOptions = options.map((option) => ({ ...option, disabled: option.value === afterVersionId }));
  const afterOptions = options.map((option) => ({ ...option, disabled: option.value === beforeVersionId }));

  return (
    <section className="file-version-diff" aria-labelledby={titleId}>
      <header>
        <div>
          <h3 id={titleId}>{t("fileSpace.timeline.diff.title")}</h3>
          <p>{t("fileSpace.timeline.diff.description")}</p>
        </div>
        {status === "ready" && result && !result.identical ? (
          <div className="file-version-diff-stats" aria-label={t("fileSpace.timeline.diff.statsLabel")}>
            <span className="is-added">+{result.addedLines}</span>
            <span className="is-removed">−{result.removedLines}</span>
          </div>
        ) : null}
      </header>

      {versions.length >= 2 && beforeVersionId && afterVersionId ? (
        <div className="file-version-diff-controls">
          <MacSelect
            className="file-version-diff-version-select"
            value={beforeVersionId}
            options={beforeOptions}
            ariaLabel={t("fileSpace.timeline.diff.beforeVersion")}
            menuMinWidth={158}
            onChange={onChangeBefore}
          />
          <button
            className="file-version-diff-swap"
            type="button"
            onClick={onSwap}
            title={t("fileSpace.timeline.diff.swap")}
            aria-label={t("fileSpace.timeline.diff.swap")}
          >
            <ArrowLeftRight size={15} />
          </button>
          <MacSelect
            className="file-version-diff-version-select"
            value={afterVersionId}
            options={afterOptions}
            ariaLabel={t("fileSpace.timeline.diff.afterVersion")}
            menuAlign="end"
            menuMinWidth={158}
            onChange={onChangeAfter}
          />
          <div className="file-version-diff-view-switch" role="group" aria-label={t("fileSpace.timeline.diff.viewMode")}>
            <button
              type="button"
              className={viewMode === "unified" ? "is-active" : ""}
              aria-pressed={viewMode === "unified"}
              title={t("fileSpace.timeline.diff.unified")}
              onClick={() => setViewMode("unified")}
            ><Rows3 size={15} /></button>
            <button
              type="button"
              className={viewMode === "split" ? "is-active" : ""}
              aria-pressed={viewMode === "split"}
              title={t("fileSpace.timeline.diff.split")}
              onClick={() => setViewMode("split")}
            ><Columns2 size={15} /></button>
          </div>
        </div>
      ) : null}

      <div className="file-version-diff-content" aria-live="polite">
        {versions.length < 2 ? (
          <div className="file-version-diff-state"><span>{t("fileSpace.timeline.diff.needTwoVersions")}</span></div>
        ) : status === "loading" ? (
          <div className="file-version-diff-state" role="status"><LoaderCircle className="is-spinning" size={18} /><span>{t("fileSpace.timeline.diff.loading")}</span></div>
        ) : status === "unsupported" ? (
          <div className="file-version-diff-state"><AlertTriangle size={18} /><span>{t("fileSpace.timeline.diff.unsupported")}</span></div>
        ) : status === "tooLarge" ? (
          <div className="file-version-diff-state"><AlertTriangle size={18} /><span>{t("fileSpace.timeline.diff.tooLarge")}</span></div>
        ) : status === "tooManyChanges" ? (
          <div className="file-version-diff-state"><AlertTriangle size={18} /><span>{t("fileSpace.timeline.diff.tooManyChanges")}</span></div>
        ) : status === "error" ? (
          <div className="file-version-diff-state is-error" role="alert"><AlertTriangle size={18} /><strong>{t("fileSpace.timeline.diff.loadError")}</strong>{error ? <span>{error}</span> : null}<button type="button" onClick={onRetry}>{t("fileSpace.timeline.diff.retry")}</button></div>
        ) : status === "ready" && result?.identical ? (
          <div className="file-version-diff-state"><span>{t("fileSpace.timeline.diff.noChanges", { before: beforeVersion?.versionNumber, after: afterVersion?.versionNumber })}</span></div>
        ) : status === "ready" && result ? (
          <div className={`file-version-diff-table is-${viewMode}`} role="table" aria-label={t("fileSpace.timeline.diff.resultLabel")}>
            {viewMode === "split" ? (
              <div className="file-version-diff-split-heading" role="row">
                <strong>{t("fileSpace.timeline.diff.version", { version: beforeVersion?.versionNumber })}</strong>
                <strong>{t("fileSpace.timeline.diff.version", { version: afterVersion?.versionNumber })}</strong>
              </div>
            ) : null}
            {result.rows.map((row, index) => row.kind === "omitted" ? (
              <div className="file-version-diff-omitted" key={`omitted:${index}`} role="row">
                {t("fileSpace.timeline.diff.omitted", { count: row.omittedLines })}
              </div>
            ) : viewMode === "unified" ? (
              <UnifiedRow key={`row:${index}`} row={row} />
            ) : (
              <SplitRow key={`row:${index}`} row={row} />
            ))}
          </div>
        ) : (
          <div className="file-version-diff-state"><span>{t("fileSpace.timeline.diff.chooseVersions")}</span></div>
        )}
      </div>
    </section>
  );
}

import { AlertTriangle, History, LoaderCircle } from "lucide-react";

export interface PreviewTaskFileVersion {
  id: string;
  versionNumber: number;
  origin: "task" | "user_edit";
  taskTitle: string | null;
  roundNumber: number | null;
  cellName: string | null;
  producedAt: number;
  isCurrent: boolean;
}

interface TaskVersionTimelineRailProps {
  versions: PreviewTaskFileVersion[] | null;
  versionCount: number;
  selectedVersionId: string | null;
  loading: boolean;
  error: string | null;
  disabled?: boolean;
  locale: string;
  onSelect: (version: PreviewTaskFileVersion) => void;
  onRetry: () => void;
  copy: {
    title: string;
    count: (count: number) => string;
    loading: string;
    loadError: string;
    retry: string;
    current: string;
    userEdit: string;
    round: (count: number) => string;
  };
  tone?: "default" | "dark";
}

export function TaskVersionTimelineRail({
  versions,
  versionCount,
  selectedVersionId,
  loading,
  error,
  disabled = false,
  locale,
  onSelect,
  onRetry,
  copy,
  tone = "default",
}: TaskVersionTimelineRailProps) {
  return (
    <aside className={`file-task-version-rail is-${tone}`} aria-label={copy.title}>
      <header>
        <div><History size={15} /><span>{copy.title}</span></div>
        <small>{copy.count(versions?.length ?? versionCount)}</small>
      </header>
      {loading ? (
        <div className="file-task-version-state" role="status"><LoaderCircle className="is-spinning" size={17} /><span>{copy.loading}</span></div>
      ) : error ? (
        <div className="file-task-version-state is-error" role="alert">
          <AlertTriangle size={17} /><strong>{copy.loadError}</strong><span>{error}</span><button type="button" onClick={onRetry}>{copy.retry}</button>
        </div>
      ) : versions ? (
        <ol className="file-task-version-list">
          {versions.map((version) => (
            <li key={version.id} className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}>
              <button type="button" aria-pressed={selectedVersionId === version.id} disabled={disabled} onClick={() => onSelect(version)}>
                <time dateTime={new Date(version.producedAt).toISOString()}>{new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(version.producedAt)}</time>
                <strong>v{version.versionNumber}{version.isCurrent ? <span>{copy.current}</span> : null}</strong>
                <span>{version.cellName ?? (version.origin === "user_edit" ? copy.userEdit : version.taskTitle)}</span>
                {version.roundNumber ? <small>{copy.round(version.roundNumber)}</small> : null}
              </button>
            </li>
          ))}
        </ol>
      ) : null}
    </aside>
  );
}

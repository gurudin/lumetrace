import { Clock3, File, FolderOpen, Info, Tag } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { InitialVersionHint, VersionName } from "./InitialVersionHint";

export interface InspectorFileRecord {
  id: string;
  name: string;
  relativePath: string;
  mimeType: string | null;
  sizeBytes: number;
  sourceKind: string;
  currentVersion: number | null;
  versionCount: number;
  tags: string[];
  updatedAt: number;
}

export interface InspectorTimelineRecord {
  fileId: string;
  versions: Array<{
    id: string;
    versionNumber: number;
    origin: "task" | "user_edit";
    taskTitle: string | null;
    producedAt: number;
    isCurrent: boolean;
  }>;
}

interface FileSpaceInspectorProps {
  hidden?: boolean;
  files: InspectorFileRecord[];
  artworks: ReactNode[];
  folderNames: string[];
  timeline: InspectorTimelineRecord | null;
  timelineLoading: boolean;
  formatFileSize: (bytes: number) => string;
  onShowTimeline: () => void;
  onReveal: () => void;
  onEditTags: () => void;
}

export function FileSpaceInspector({
  hidden = false,
  files,
  artworks,
  folderNames,
  timeline,
  timelineLoading,
  formatFileSize,
  onShowTimeline,
  onReveal,
  onEditTags,
}: FileSpaceInspectorProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? "en-US";
  const dateFormatter = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" });
  const file = files.length === 1 ? files[0] : null;
  const artwork = artworks[0] ?? null;
  const currentTimeline = file && timeline?.fileId === file.id ? timeline : null;
  const currentTimelineLoading = Boolean(file && timelineLoading && (!timeline || timeline.fileId === file.id));
  const versionCount = currentTimeline?.versions.length ?? file?.versionCount ?? 0;
  const multiple = files.length > 1;
  const totalSize = files.reduce((total, selected) => total + selected.sizeBytes, 0);
  const totalVersions = files.reduce((total, selected) => total + selected.versionCount, 0);
  const latestUpdated = files.reduce((latest, selected) => Math.max(latest, selected.updatedAt), 0);
  const fileTypes = [...new Set(files.map((selected) => {
    const extension = selected.name.includes(".") ? selected.name.split(".").pop() : null;
    return (extension || selected.mimeType?.split("/").pop() || "—").toLocaleUpperCase(locale);
  }))];
  const sources = [...new Set(files.map((selected) => (
    selected.sourceKind === "task_artifact"
      ? t("fileSpace.content.taskArtifact")
      : t("fileSpace.content.userImport")
  )))];
  const commonTags = multiple
    ? files[0].tags.filter((tagName) => files.every((selected) => selected.tags.includes(tagName)))
    : [];

  return (
    <aside className="file-space-inspector" aria-label={t("fileSpace.inspector.title")} aria-hidden={hidden} inert={hidden}>
      <header data-tauri-drag-region>
        <strong data-tauri-drag-region>{t("fileSpace.inspector.title")}</strong>
      </header>
      {files.length === 0 ? (
        <div className="file-space-inspector-empty">
          <span><Info size={23} /></span>
          <strong>{t("fileSpace.inspector.emptyTitle")}</strong>
          <p>{t("fileSpace.inspector.emptyDescription")}</p>
        </div>
      ) : multiple ? (
        <div className="file-space-inspector-scroll">
          <section className="file-space-inspector-hero is-multiple">
            <div className="file-space-inspector-artwork-stack" aria-hidden="true">
              {artworks.slice(0, 4).map((selectedArtwork, index) => (
                <div
                  className="file-space-inspector-stack-item"
                  key={files[index]?.id ?? index}
                  style={{ "--stack-index": index } as React.CSSProperties}
                >
                  {selectedArtwork}
                </div>
              ))}
            </div>
            <h2>{t("fileSpace.inspector.multipleTitle", { count: files.length })}</h2>
          </section>

          <section className="file-space-inspector-section">
            <h3>{t("fileSpace.inspector.information")}</h3>
            <dl>
              <div><dt><File size={15} />{t("fileSpace.inspector.totalSize")}</dt><dd>{formatFileSize(totalSize)}</dd></div>
              <div><dt><Clock3 size={15} />{t("fileSpace.inspector.totalVersions")}</dt><dd>{totalVersions}</dd></div>
              <div><dt><File size={15} />{t("fileSpace.inspector.types")}</dt><dd title={fileTypes.join(", ")}>{fileTypes.join(", ")}</dd></div>
              <div><dt><FolderOpen size={15} />{t("fileSpace.inspector.folders")}</dt><dd title={folderNames.join(", ")}>{folderNames.join(", ")}</dd></div>
              <div><dt><File size={15} />{t("fileSpace.inspector.sources")}</dt><dd title={sources.join(", ")}>{sources.join(", ")}</dd></div>
              <div><dt><Clock3 size={15} />{t("fileSpace.inspector.latestUpdated")}</dt><dd>{dateFormatter.format(latestUpdated)}</dd></div>
            </dl>
          </section>

          <section className="file-space-inspector-section">
            <h3>{t("fileSpace.inspector.commonTags")}</h3>
            {commonTags.length ? (
              <div className="file-space-inspector-tags">
                {commonTags.map((tagName) => <span key={tagName}>{tagName}</span>)}
              </div>
            ) : <p className="file-space-inspector-muted">{t("fileSpace.inspector.noCommonTags")}</p>}
          </section>
        </div>
      ) : file ? (
        <div className="file-space-inspector-scroll">
          <section className="file-space-inspector-hero">
            <div className="file-space-inspector-artwork" style={{ "--file-preview-size": "112px" } as React.CSSProperties}>{artwork}</div>
            <h2 title={file.name}>{file.name}</h2>
          </section>

          <div className="file-space-inspector-actions">
            <button type="button" onClick={onReveal}><FolderOpen size={16} /><span>{t("fileSpace.inspector.reveal")}</span></button>
            <button type="button" onClick={onEditTags}><Tag size={16} /><span>{t("fileSpace.inspector.tags")}</span></button>
            <button type="button" disabled={versionCount < 1} onClick={onShowTimeline}><Clock3 size={16} /><span>{t("fileSpace.inspector.history")}</span></button>
          </div>

          <section className="file-space-inspector-section">
            <h3>{t("fileSpace.inspector.information")}</h3>
            <dl>
              <div><dt><Clock3 size={15} />{t("fileSpace.inspector.versions")}</dt><dd>{currentTimelineLoading ? t("fileSpace.inspector.loading") : versionCount}</dd></div>
              <div><dt><Clock3 size={15} />{t("fileSpace.inspector.updated")}</dt><dd>{dateFormatter.format(file.updatedAt)}</dd></div>
              <div><dt><File size={15} />{t("fileSpace.inspector.size")}</dt><dd>{formatFileSize(file.sizeBytes)}</dd></div>
              <div><dt><File size={15} />{t("fileSpace.inspector.source")}</dt><dd>{file.sourceKind === "task_artifact" ? t("fileSpace.content.taskArtifact") : t("fileSpace.content.userImport")}</dd></div>
              <div className="is-path"><dt><FolderOpen size={15} />{t("fileSpace.inspector.location")}</dt><dd title={file.relativePath}>{file.relativePath}</dd></div>
            </dl>
          </section>

          <section className="file-space-inspector-section">
            <h3>{t("fileSpace.inspector.tagsHeading")}</h3>
            {file.tags.length ? (
              <div className="file-space-inspector-tags">
                {file.tags.map((tagName) => <span key={tagName}>{tagName}</span>)}
              </div>
            ) : <p className="file-space-inspector-muted">{t("fileSpace.inspector.noTags")}</p>}
          </section>

          <section className="file-space-inspector-section file-space-inspector-activity">
            <h3>{t("fileSpace.inspector.recentVersions")}</h3>
            {currentTimelineLoading ? <p className="file-space-inspector-muted">{t("fileSpace.inspector.loading")}</p> : currentTimeline?.versions.length ? (
              <ol>
                {currentTimeline.versions.slice(0, 4).map((version) => (
                  <li key={version.id}>
                    <Clock3 size={15} />
                    <span>
                      <strong><VersionName number={version.versionNumber} /> · {version.origin === "task" ? (version.taskTitle ?? t("fileSpace.content.taskArtifact")) : t("fileSpace.timeline.userEdit")}</strong>
                      <small>{dateFormatter.format(version.producedAt)}</small>
                    </span>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="file-space-inspector-muted">{t("fileSpace.inspector.noVersions")}</p>
            )}
            {!currentTimelineLoading ? <InitialVersionHint versions={currentTimeline?.versions ?? null} /> : null}
          </section>
        </div>
      ) : null}
    </aside>
  );
}

import { File, Folder, LoaderCircle, RotateCcw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

export interface FileSpaceTrashItemRecord {
  id: string;
  rootId: string;
  itemType: "file" | "folder";
  name: string;
  originalRelativePath: string;
  sizeBytes: number;
  fileCount: number;
  folderCount: number;
  versionCount: number;
  trashedAt: number;
}

interface FileSpaceTrashViewProps {
  items: FileSpaceTrashItemRecord[];
  restoringId: string | null;
  busy: boolean;
  locale: string;
  retentionMs: number;
  formatFileSize: (bytes: number) => string;
  onRestore: (entryId: string, focusEntryId: string | null) => void;
}

export function FileSpaceTrashView({
  items,
  restoringId,
  busy,
  locale,
  retentionMs,
  formatFileSize,
  onRestore,
}: FileSpaceTrashViewProps) {
  const { t } = useTranslation();

  if (items.length === 0) {
    return (
      <div className="file-space-trash-empty">
        <span aria-hidden="true"><Trash2 size={28} strokeWidth={1.35} /></span>
        <strong>{t("fileSpace.trash.emptyTitle")}</strong>
        <p>{t("fileSpace.trash.emptyDescription")}</p>
      </div>
    );
  }

  return (
    <div className="file-space-trash-list" role="list" aria-label={t("fileSpace.trash.title")} aria-busy={busy}>
      {items.map((item, index) => {
        const ItemIcon = item.itemType === "folder" ? Folder : File;
        const restoring = restoringId === item.id;
        const details = item.itemType === "folder"
          ? [t("fileSpace.trash.folderContentsPreserved")]
          : [
              formatFileSize(item.sizeBytes),
              item.versionCount > 0
                ? t("fileSpace.trash.versionCount", { count: item.versionCount })
                : null,
            ];
        return (
          <article className="file-space-trash-item" role="listitem" key={item.id}>
            <span className={`file-space-trash-item-icon is-${item.itemType}`} aria-hidden="true">
              <ItemIcon size={24} strokeWidth={1.35} />
            </span>
            <div className="file-space-trash-item-copy">
              <strong title={item.name}>{item.name}</strong>
              <span>{details.filter(Boolean).join(" · ")}</span>
              <small title={item.originalRelativePath}>
                {t("fileSpace.trash.originalLocation", { path: item.originalRelativePath })}
              </small>
            </div>
            <div className="file-space-trash-dates">
              <time dateTime={new Date(item.trashedAt).toISOString()}>
                {t("fileSpace.trash.deletedAt", {
                  date: new Intl.DateTimeFormat(locale, {
                    dateStyle: "medium",
                    timeStyle: "short",
                  }).format(item.trashedAt),
                })}
              </time>
              <time dateTime={new Date(item.trashedAt + retentionMs).toISOString()}>
                {t("fileSpace.trash.autoDeleteAt", {
                  date: new Intl.DateTimeFormat(locale, {
                    dateStyle: "medium",
                  }).format(item.trashedAt + retentionMs),
                })}
              </time>
            </div>
            <button
              className="file-space-trash-restore"
              type="button"
              disabled={busy || Boolean(restoringId)}
              data-trash-entry-restore={item.id}
              aria-busy={restoring}
              aria-label={t("fileSpace.trash.restoreItem", { name: item.name })}
              onClick={() => onRestore(
                item.id,
                items[index + 1]?.id ?? items[index - 1]?.id ?? null,
              )}
            >
              {restoring ? <LoaderCircle className="is-spinning" size={15} /> : <RotateCcw size={15} />}
              {restoring ? t("fileSpace.trash.restoring") : t("fileSpace.trash.restore")}
            </button>
          </article>
        );
      })}
    </div>
  );
}

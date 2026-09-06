import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import "./preview-file-heading.css";

interface PreviewFileHeadingProps {
  name: string;
  versionCount: number;
  children?: ReactNode;
}

export function PreviewFileHeading({ name, versionCount, children }: PreviewFileHeadingProps) {
  const { t } = useTranslation();
  const summary = t("fileSpace.preview.common.fileVersionCount", { count: versionCount });

  return (
    <div className="file-preview-heading">
      <div className="file-preview-heading-row">
        <strong title={name}>{name}</strong>
        {children}
      </div>
      <span className="file-preview-heading-summary" title={summary}>{summary}</span>
    </div>
  );
}

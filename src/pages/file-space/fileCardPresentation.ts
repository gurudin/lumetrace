export type FileDocumentArtworkFormat = "md" | "pdf" | "html" | "xlsx" | "csv" | "docx" | "txt";

export interface FileImageDimensions {
  fileUpdatedAt: number;
  width: number;
  height: number;
}

export function fileDocumentArtworkFormat(extension: string): FileDocumentArtworkFormat | null {
  const normalizedExtension = extension.toLowerCase();
  if (normalizedExtension === "md" || normalizedExtension === "markdown") return "md";
  return (["pdf", "html", "xlsx", "csv", "docx", "txt"] as const)
    .find((format) => format === normalizedExtension) ?? null;
}

export function shouldShowFileVersionBadge(versionCount: number) {
  return versionCount > 1;
}

export function fileCardMetadataText(
  sizeText: string,
  isImage: boolean,
  fileUpdatedAt: number,
  imageDimensions?: FileImageDimensions,
) {
  if (!isImage || imageDimensions?.fileUpdatedAt !== fileUpdatedAt) return sizeText;
  return `${sizeText} · ${imageDimensions.width} × ${imageDimensions.height}`;
}

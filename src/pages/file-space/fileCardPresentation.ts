export type FileDocumentArtworkFormat = "md" | "pdf" | "html" | "xlsx" | "csv" | "docx" | "txt";

export interface FileImageDimensions {
  fileUpdatedAt: number;
  width: number;
  height: number;
}

export function originalImageDimensions(value: unknown): { width: number; height: number } | null {
  const dimensions = value as { width?: unknown; height?: unknown } | null;
  if (!dimensions || !Number.isSafeInteger(dimensions.width) || !Number.isSafeInteger(dimensions.height)
    || (dimensions.width as number) <= 0 || (dimensions.height as number) <= 0) return null;
  return { width: dimensions.width as number, height: dimensions.height as number };
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

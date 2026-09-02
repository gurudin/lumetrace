export type FileDoubleClickRoute = "internal-preview" | "external-open" | "reveal";

const internalDocumentPreviewPattern = /\.(pdf|md|markdown|txt)$/i;
const externalDocumentPattern = /\.(doc|docx|xls|xlsx|ppt|pptx|html?|csv)$/i;

export function resolveFileDoubleClickRoute(
  fileName: string,
  hasImagePreview = false,
): FileDoubleClickRoute {
  const normalizedName = fileName.trim();
  if (hasImagePreview || internalDocumentPreviewPattern.test(normalizedName)) {
    return "internal-preview";
  }
  return externalDocumentPattern.test(normalizedName) ? "external-open" : "reveal";
}

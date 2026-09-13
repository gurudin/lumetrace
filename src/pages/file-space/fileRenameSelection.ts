/** Select the basename only, while leaving the full filename editable. */
export function fileRenameSelectionEnd(name: string): number {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? dot : name.length;
}

function escapeAttribute(value: string) {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/** Reuse the complete startup/theme shell; change only edition-owned metadata. */
export function renderApplicationHtml(
  template: string,
  options: { displayName: string; assetPrefix: string },
) {
  return template
    .replaceAll("Lume Trace", escapeAttribute(options.displayName))
    .replace('src="/src-tauri/icons/icon.png"',
      `src="${escapeAttribute(options.assetPrefix)}/src-tauri/icons/icon.png"`);
}

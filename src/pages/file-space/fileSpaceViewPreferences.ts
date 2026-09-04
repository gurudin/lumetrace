interface PreferenceReader {
  getItem(key: string): string | null;
}

// Version 2 changes the first-run inspector default from open to closed.
// A new key prevents the legacy default-open value from being mistaken for
// an explicit preference, while choices made from now on remain persistent.
export const inspectorVisibilityStorageKey = "lumetrace.file-space.inspector-visible.v2";

export function storedInspectorVisibility(storage: PreferenceReader) {
  return storage.getItem(inspectorVisibilityStorageKey) === "true";
}

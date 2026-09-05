import type { FileSpaceVersionNotification } from "./versionNotification.ts";

export const firstVersionChangeStorageKey = "lumetrace.onboarding.first-version-change-seen";
export const viewVersionChangeEvent = "lumetrace:view-version-change";

// One app-wide claim, shared by external-change notifications and the Markdown editor.
// Memory is also retained if localStorage is unavailable.
export function createFirstVersionChangeClaim(storage: () => Pick<Storage, "getItem" | "setItem">) {
  let claimed = false;
  return (notification: FileSpaceVersionNotification) => {
    if (notification.versionNumber < 2 || claimed) return false;
    try {
      if (storage().getItem(firstVersionChangeStorageKey) === "1") {
        claimed = true;
        return false;
      }
      storage().setItem(firstVersionChangeStorageKey, "1");
    } catch {
      // A storage failure must never turn a successful save into an error.
    }
    claimed = true;
    return true;
  };
}

export const claimFirstVersionChange = createFirstVersionChangeClaim(() => window.localStorage);

export function savedVersionNotification(
  fileId: string,
  fileName: string,
  previousVersionNumber: number,
  versions: readonly { id: string; versionNumber: number; isCurrent: boolean; origin: string; producedAt: number }[],
): FileSpaceVersionNotification | null {
  const version = versions.find((candidate) => candidate.isCurrent);
  if (!version || version.origin !== "user_edit" || version.versionNumber < 2
    || version.versionNumber <= previousVersionNumber) return null;
  return { fileId, fileName, versionId: version.id, versionNumber: version.versionNumber, createdAt: version.producedAt };
}

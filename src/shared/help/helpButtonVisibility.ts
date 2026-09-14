export const helpButtonVisibilityStorageKey = "lumetrace.help.button.visible";
export const helpButtonVisibilityChangeEvent = "lumetrace:help-button-visibility-change";

type StorageReader = Pick<Storage, "getItem">;

export function storedHelpButtonVisibility(storage: StorageReader): boolean {
  return storage.getItem(helpButtonVisibilityStorageKey) !== "false";
}

export function getHelpButtonVisibility(): boolean {
  return storedHelpButtonVisibility(window.localStorage);
}

export function setHelpButtonVisibility(visible: boolean): void {
  window.localStorage.setItem(helpButtonVisibilityStorageKey, String(visible));
  window.dispatchEvent(new CustomEvent(helpButtonVisibilityChangeEvent, {
    detail: { visible },
  }));
}

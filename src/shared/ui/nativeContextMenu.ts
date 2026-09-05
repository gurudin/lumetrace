export type NativeContextMenuTargetKind =
  | "input"
  | "textarea"
  | "contenteditable"
  | "selectable-text"
  | "other";

export function shouldPreserveNativeContextMenu(kind: NativeContextMenuTargetKind) {
  return kind !== "other";
}

export function nativeContextMenuTargetKind(target: EventTarget | null): NativeContextMenuTargetKind {
  if (!(target instanceof Element)) return "other";

  if (target.closest("input")) return "input";
  if (target.closest("textarea")) return "textarea";

  const htmlTarget = target instanceof HTMLElement ? target : target.parentElement;
  if (htmlTarget?.isContentEditable) return "contenteditable";

  if (target.closest('[data-native-context-menu="true"]')) return "selectable-text";
  return "other";
}

export function shouldPreserveNativeContextMenuForTarget(target: EventTarget | null) {
  return shouldPreserveNativeContextMenu(nativeContextMenuTargetKind(target));
}

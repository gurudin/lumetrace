export function toggleFolderOnDoubleClick(
  expandedFolders: Set<string>,
  folderId: string,
  hasChildren: boolean,
): Set<string> {
  if (!hasChildren) return expandedFolders;
  const next = new Set(expandedFolders);
  if (next.has(folderId)) next.delete(folderId);
  else next.add(folderId);
  return next;
}

export interface FolderTreeNode {
  id: string;
  parentId: string | null;
}

export function expandableFolderIdsInSubtree(
  folders: FolderTreeNode[],
  rootId: string,
): string[] {
  const childrenByParent = new Map<string, string[]>();
  folders.forEach((folder) => {
    if (!folder.parentId) return;
    const children = childrenByParent.get(folder.parentId) ?? [];
    children.push(folder.id);
    childrenByParent.set(folder.parentId, children);
  });

  const expandableIds: string[] = [];
  const pendingIds = [rootId];
  const visitedIds = new Set<string>();
  while (pendingIds.length > 0) {
    const folderId = pendingIds.pop();
    if (!folderId || visitedIds.has(folderId)) continue;
    visitedIds.add(folderId);
    const childIds = childrenByParent.get(folderId) ?? [];
    if (childIds.length === 0) continue;
    expandableIds.push(folderId);
    pendingIds.push(...childIds);
  }
  return expandableIds;
}

export function isFolderSubtreeFullyExpanded(
  expandedFolders: Set<string>,
  expandableFolderIds: string[],
): boolean {
  return expandableFolderIds.length > 0
    && expandableFolderIds.every((folderId) => expandedFolders.has(folderId));
}

export function setFolderSubtreeExpanded(
  expandedFolders: Set<string>,
  expandableFolderIds: string[],
  expanded: boolean,
): Set<string> {
  const next = new Set(expandedFolders);
  expandableFolderIds.forEach((folderId) => {
    if (expanded) next.add(folderId);
    else next.delete(folderId);
  });
  return next;
}

export interface FileSpaceVersionNotification {
  versionId: string;
  fileId: string;
  fileName: string;
  versionNumber: number;
  createdAt: number;
}

const notificationLimit = 100;

export function mergeVersionNotifications(
  current: readonly FileSpaceVersionNotification[],
  incoming: readonly FileSpaceVersionNotification[],
) {
  if (incoming.length === 0) return [...current];
  const seen = new Set(current.map((notification) => notification.versionId));
  const merged = [...current];
  for (const notification of incoming) {
    if (seen.has(notification.versionId)) continue;
    seen.add(notification.versionId);
    merged.push(notification);
  }
  return merged.slice(-notificationLimit);
}

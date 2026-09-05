export interface RevealableFile {
  id: string;
  folderId: string | null;
}

export interface FileRevealRequest<T extends RevealableFile> {
  file: T;
  open: boolean;
  pageReady: boolean;
}

// A reveal belongs to one navigation, not to a fixed number of animation frames.
// Keep its target pinned until the destination page AND virtualized card exist.
export class FileRevealNavigation<T extends RevealableFile> {
  pending: FileRevealRequest<T> | null = null;

  start(file: T, open = false, pageReady = false) {
    const request = { file, open, pageReady };
    this.pending = request;
    return request;
  }

  cancel() {
    this.pending = null;
  }

  acceptPage(request: FileRevealRequest<T> | null, folderId: string | null, files: T[]) {
    if (!request || request !== this.pending || request.file.folderId !== folderId) return files;
    request.pageReady = true;
    return files.some((file) => file.id === request.file.id) ? files : [request.file, ...files];
  }

  readyForLayout(folderId: string | null) {
    const request = this.pending;
    return request?.pageReady && request.file.folderId === folderId ? request : null;
  }

  completeWithTarget<Target>(
    request: FileRevealRequest<T>,
    target: Target | null,
    focus: (target: Target) => void,
    open: (target: Target) => void,
  ) {
    if (request !== this.pending || !request.pageReady || !target) return false;
    this.pending = null;
    focus(target);
    if (request.open) open(target);
    return true;
  }
}

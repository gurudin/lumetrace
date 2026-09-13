export interface RevealableFile {
  id: string;
  folderId: string | null;
}

export interface FileRevealRequest<T extends RevealableFile, Context = undefined> {
  file: T;
  open: boolean;
  pageReady: boolean;
  openContext: Context | undefined;
}

// A reveal belongs to one navigation, not to a fixed number of animation frames.
// Keep its target pinned until the destination page AND virtualized card exist.
export class FileRevealNavigation<T extends RevealableFile, Context = undefined> {
  pending: FileRevealRequest<T, Context> | null = null;

  start(file: T, open = false, pageReady = false, openContext?: Context) {
    const request = { file, open, pageReady, openContext };
    this.pending = request;
    return request;
  }

  cancel() {
    this.pending = null;
  }

  acceptPage(request: FileRevealRequest<T, Context> | null, folderId: string | null, files: T[]) {
    if (!request || request !== this.pending || request.file.folderId !== folderId) return files;
    request.pageReady = true;
    return files.some((file) => file.id === request.file.id) ? files : [request.file, ...files];
  }

  readyForLayout(folderId: string | null) {
    const request = this.pending;
    return request?.pageReady && request.file.folderId === folderId ? request : null;
  }

  completeWithTarget<Target>(
    request: FileRevealRequest<T, Context>,
    target: Target | null,
    focus: (target: Target) => void,
    open: (target: Target, context: Context | undefined) => void,
  ) {
    if (request !== this.pending || !request.pageReady || !target) return false;
    this.pending = null;
    focus(target);
    if (request.open) open(target, request.openContext);
    return true;
  }
}

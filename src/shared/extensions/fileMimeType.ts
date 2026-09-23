/** Filename fallback for catalogues that do not supply a Content-Type. */
export function fileMimeType(name: string): string | null {
  const extension = name.split('.').pop()?.toLowerCase() ?? '';
  const types: Record<string, string> = {
    png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp',
    svg: 'image/svg+xml', bmp: 'image/bmp', heic: 'image/heic', tiff: 'image/tiff',
    md: 'text/markdown', markdown: 'text/markdown', txt: 'text/plain', csv: 'text/csv',
    pdf: 'application/pdf', doc: 'application/msword', docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    xls: 'application/vnd.ms-excel', xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    csr: 'application/pkcs10', p12: 'application/x-pkcs12', cer: 'application/pkix-cert',
    mp4: 'video/mp4', mov: 'video/quicktime', m4v: 'video/x-m4v', webm: 'video/webm',
    mkv: 'video/x-matroska', avi: 'video/x-msvideo', zip: 'application/zip',
  };
  return types[extension] ?? null;
}

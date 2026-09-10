/** Availability is supplied by the transport, never inferred from displayed dimensions. */
export function originalImageSource(preview: string, original?: string | null) {
  return original && original !== preview ? original : null;
}

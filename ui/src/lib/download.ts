/**
 * Trigger a client-side file download of `text` named `name` (the
 * `ArtifactGallery` helper, lifted to a shared module so chat export + artifact
 * download share ONE implementation). A Blob URL + a transient anchor click;
 * revoked immediately after.
 */
export function download(name: string, text: string, type = "text/plain"): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Download raw `bytes` — the binary twin of {@link download} for media/binary
 * artifacts (an image/video/audio blob, never its lossy hex preview). Same
 * transient-anchor + immediate-revoke discipline.
 */
export function downloadBytes(
  name: string,
  bytes: Uint8Array,
  type = "application/octet-stream",
): void {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

// Validate absolute native paths returned by the desktop process.
export function displayNativePath(value: string | null | undefined): string {
  if (!value) return "";
  if (/^\\\\\?\\[A-Za-z]:\\/u.test(value)) return value.slice(4);
  if (/^\\\\\?\\UNC\\/iu.test(value)) return `\\\\${value.slice(8)}`;
  return value;
}

export function nativeAbsolutePath(value: unknown, limit = 4096): value is string {
  return typeof value === "string" && value.length <= limit && !value.includes("\0")
    && (/^\//u.test(value) || /^[A-Za-z]:[\\/]/u.test(value) || /^\\\\\?\\[A-Za-z]:\\/u.test(value));
}

// Validate absolute native paths returned by the desktop process.
export function displayNativePath(value: string | null | undefined): string {
  if (!value) return "";
  if (/^\\\\\?\\[A-Za-z]:\\/u.test(value)) return value.slice(4);
  if (/^\\\\\?\\UNC\\/iu.test(value)) return `\\\\${value.slice(8)}`;
  return value;
}

/** Format native paths embedded in diagnostics, keeping JSON/Rust Debug escaping.
 * Source diagnostics and native operation arguments must retain their spelling. */
export function displayNativePathsInText(value: string): string {
  return value.replace(/(\\+)\?(\\+)(?:[A-Za-z]:(\\+)|UNC(\\+))/giu, (prefix, leading: string, separator: string, driveRoot?: string, uncRoot?: string) => {
    const width = separator.length;
    if (leading.length !== width * 2 || (driveRoot ?? uncRoot)?.length !== width || !Number.isInteger(Math.log2(width))) return prefix;
    const escapedSlash = "\\".repeat(width);
    return displayNativePath(prefix.replaceAll(escapedSlash, "\\")).replaceAll("\\", escapedSlash);
  });
}

export function nativeAbsolutePath(value: unknown, limit = 4096): value is string {
  return typeof value === "string" && value.length <= limit && !value.includes("\0")
    && (/^\//u.test(value) || /^[A-Za-z]:[\\/]/u.test(value) || /^\\\\\?\\[A-Za-z]:\\/u.test(value));
}

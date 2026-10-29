import { invoke as nativeInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { assertCommandArguments } from "./native-command-state.ts";
import { normalizeAppError } from "./app-error.ts";
export * from "@tauri-apps/api/core";

/** Preserve native error metadata while making existing Error/String consumers
 * render the user-facing message at the single IPC boundary. */
export async function invoke<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  assertCommandArguments(command, args);
  try { return await nativeInvoke<T>(command, args, options); }
  catch (cause) { throw normalizeAppError(cause); }
}

import type { AppError } from "./generated/contracts";
import { validateAppError } from "./generated/validators.js";
export function appError(value: unknown): AppError | null {
  return validateAppError(value) ? value : null;
}
export function errorMessage(value: unknown): string {
  return appError(value)?.message ?? String(value);
}
export function normalizeAppError(value: unknown): unknown {
  const error = appError(value);
  if (!error) return value;
  // Non-enumerable Error internals keep the generated DTO validator applicable.
  return Object.assign(new Error(error.message), {
    code: error.code, operationId: error.operationId ?? null, retryDisposition: error.retryDisposition,
  });
}
export function operationNotStarted(value: unknown, code: string): boolean {
  const error = appError(value);
  return error?.code === code && error.retryDisposition === "not_started";
}
export function operationFailureMessage(value: unknown, _code?: string): string {
  return errorMessage(value);
}

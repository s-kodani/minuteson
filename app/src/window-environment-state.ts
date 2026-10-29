import type { WindowEnvironment as NativeWindowEnvironment, WindowRole } from "./generated/contracts";
import { validateWindowEnvironment } from "./generated/validators.js";
export type { WindowRole } from "./generated/contracts";
export type WindowEnvironment = NativeWindowEnvironment & { schemaVersion: 1 };
export function windowRole(search: string): WindowRole {
  const value = new URLSearchParams(search).get("window") ?? "main";
  if (!["main", "recorder", "recorder-info", "recording-confirmation", "startup-attention"].includes(value)) throw Error("ウィンドウの役割を確認できません。");
  return value as WindowRole;
}
export function checkedWindowEnvironment(value: unknown, expectedRole: WindowRole): WindowEnvironment {
  if (!validateWindowEnvironment(value) || value.schemaVersion !== 1 || value.role !== expectedRole) throw Error("ウィンドウの接続情報が一致しません。");
  return value as WindowEnvironment;
}

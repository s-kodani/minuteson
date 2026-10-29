import { commandContracts } from "./generated/commands.ts";

type Argument = { wireName: string; optional: boolean };
const contracts = new Map((Object.entries(commandContracts) as [string, { arguments: readonly Argument[] }][]).map(([name, contract]) => [name, {
  allowed: new Set(contract.arguments.map(argument => argument.wireName)),
  required: contract.arguments.filter(argument => !argument.optional).map(argument => argument.wireName),
}]));

/** Validate command/key ownership from actual Rust signatures. Domain DTO values
 * are validated by their generated schema and native command boundaries. */
export function assertCommandArguments(command: string, args?: unknown): void {
  const contract = contracts.get(command);
  if (!contract) throw Error(`未登録のアプリ操作です: ${command}`);
  if (args !== undefined && (!args || typeof args !== "object" || Array.isArray(args) || ArrayBuffer.isView(args) || args instanceof ArrayBuffer)) throw Error(`操作の引数形式が一致しません: ${command}`);
  const values = (args ?? {}) as Record<string, unknown>;
  for (const key of Reflect.ownKeys(values)) if (typeof key !== "string" || !contract.allowed.has(key)) throw Error(`操作の引数名が一致しません: ${command}`);
  for (const key of contract.required) if (!Object.hasOwn(values, key) || values[key] === undefined) throw Error(`操作に必要な引数がありません: ${command}.${key}`);
}

import test from "node:test";
import assert from "node:assert/strict";
import { assertCommandArguments } from "../src/native-command-state.ts";
import { commandContracts } from "../src/generated/commands.ts";
test("every exported Rust command admits exactly its generated required argument keys", () => {
  for (const [name, contract] of Object.entries(commandContracts)) {
    const args = Object.fromEntries(contract.arguments.filter(argument => !argument.optional).map(argument => [argument.wireName, null]));
    assert.doesNotThrow(() => assertCommandArguments(name, args), name);
    assert.throws(() => assertCommandArguments(name, {...args, unexpectedNativeArgument:true}), name);
    for (const argument of contract.arguments) if (!argument.optional) {
      const missing = {...args}; delete missing[argument.wireName];
      assert.throws(() => assertCommandArguments(name, missing), `${name}.${argument.wireName}`);
    }
  }
});
test("optional keys allow omitted and null values while injected native handles stay inaccessible", () => {
  assert.doesNotThrow(() => assertCommandArguments("select_audio_import"));
  assert.doesNotThrow(() => assertCommandArguments("select_audio_import", {path:null}));
  assert.throws(() => assertCommandArguments("get_data_operation", {window:"forged"}));
  assert.throws(() => assertCommandArguments("get_data_operation", {app:"forged"}));
  assert.throws(() => assertCommandArguments("review_meeting_deletion", {meeting_id:"wrong casing"}));
  assert.throws(() => assertCommandArguments("review_meeting_deletion", {meetingId:undefined}));
  assert.throws(() => assertCommandArguments("get_data_operation", []));
  assert.throws(() => assertCommandArguments("toString"));
  assert.throws(() => assertCommandArguments("plugin:window|close"));
});

test("the shared invoke boundary rejects contract drift before native dispatch and preserves typed native failures", async () => {
  const { invoke } = await import("../src/ipc.ts");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
  const calls: string[] = [];
  let fail = false;
  const payload = {ready:true};
  Object.defineProperty(globalThis, "window", {configurable:true,value:{__TAURI_INTERNALS__:{invoke:async (command:string) => {
    calls.push(command);
    if (fail) throw {code:"read_failed",message:"保存状態を確認できません",operationId:null,retryDisposition:"retry"};
    return payload;
  }}}});
  try {
    await assert.rejects(() => invoke("get_data_operation", {window:"forged"}));
    await assert.rejects(() => invoke("unknown_native_command"));
    assert.deepEqual(calls, []);
    assert.equal(await invoke("get_data_operation"), payload);
    fail = true;
    await assert.rejects(() => invoke("get_data_operation"), error => {
      assert.ok(error instanceof Error);
      assert.equal(error.message, "保存状態を確認できません");
      assert.equal((error as Error & {code:string}).code, "read_failed");
      assert.notEqual(String(error), "[object Object]");
      return true;
    });
    assert.deepEqual(calls, ["get_data_operation", "get_data_operation"]);
  } finally {
    if (previous) Object.defineProperty(globalThis,"window",previous);
    else Reflect.deleteProperty(globalThis,"window");
  }
});

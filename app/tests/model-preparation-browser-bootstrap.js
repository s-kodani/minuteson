// Test-only proof readable from the DOM. Does not dispatch any command.
await import("./model-preparation-browser-fixture.js");
const output = document.getElementById("model-fixture-counts");
const update = () => {
  const f = window.modelPreparationFixture;
  const count = command => f.calls.filter(call => call.command === command).length;
  output.textContent = `合成のみ・実ファイル変更なし：検査 ${count("inspect_model_package")} · コピー ${count("place_model_package")} · CPU ${count("check_model_compatibility")} · 適用 ${count("apply_model_profile_application")} · 停止 ${count("cancel_model_inspection")}`;
};
update(); setInterval(update, 200);

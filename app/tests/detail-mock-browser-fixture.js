// Existing synthetic IPC fixtures only. No native windows, audio or file writes.
if (new URLSearchParams(location.search).get("view") === "minutes") {
  const { installMinutesFixture } = await import("./minutes-browser-fixture.js");
  await installMinutesFixture({ addInitScript: async install => install(), reload: async () => {} });
  const parameters = new URLSearchParams(location.search);
  if (parameters.get("long") === "1") window.__mon.addLongDocument();
  if (parameters.get("group") === "1") window.__mon.addEvidenceGroup();
  if (parameters.get("outline") === "1") window.__mon.addOutlineDocument();
  if (parameters.get("checks") === "1") {
    const controls = document.createElement("aside");
    controls.setAttribute("aria-label", "合成の保存失敗検証");
    Object.assign(controls.style, { position: "fixed", bottom: "8px", right: "8px", zIndex: "5000", background: "white", color: "#111", padding: "6px", border: "1px solid #bbb", borderRadius: "8px", fontSize: "12px", maxWidth:"calc(100vw - 16px)" });
    controls.append("合成検証：実ファイル変更なし ");
    for (const [label, fail] of [["議事録保存失敗を合成", true], ["議事録保存を復帰", false]]) {
      const button = document.createElement("button"); button.textContent = label;
      button.onclick = () => { window.__mon.failMinutesSave = fail; };
      controls.append(button);
    }
    document.body.append(controls);
    const themeLabel = document.createElement("label"); themeLabel.textContent = "別ウィンドウのテーマ（合成） ";
    const theme = document.createElement("select"); theme.setAttribute("aria-label","別ウィンドウのテーマ（合成）");
    for (const [value,label] of [["","未変更"],["light","ライト"],["dark","ダーク"]]) { const option = document.createElement("option"); option.value=value; option.textContent=label; theme.append(option); }
    theme.onchange = () => { if (theme.value) window.__mon.changeThemeFromOtherWindow(theme.value); };
    themeLabel.append(theme); controls.append(themeLabel);
    const state = document.createElement("output"); state.setAttribute("aria-label", "合成の保存状態"); controls.append(state);
    const close = document.createElement("button"); close.textContent = "合成操作を閉じる";
    close.onclick = () => { controls.hidden = true; }; controls.append(close);
    const update = () => { state.textContent = `保存失敗: ${window.__mon.failMinutesSave ? "有効" : "無効"} · 保存要求: ${window.__mon.calls.filter(call => call.cmd === "save_minutes_draft").length} · 元発言読込: ${window.__mon.calls.filter(call => call.cmd === "get_minutes_evidence_source").length}`; };
    update(); window.setInterval(update, 250);
  }
  await import("../src/main.tsx");
} else {
  await import("./browser-fixture.js");
}

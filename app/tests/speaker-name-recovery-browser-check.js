// Production React controls with synthetic CAS storage, not macOS crash proof.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok,message) => { if(!ok) throw Error(message); };
  const editor = page.getByRole("region",{ name: "文字起こし編集",exact: true });
  const group = label => editor.getByRole("group",{ name: `${label}の名前編集`,exact: true });
  const name = label => group(label).getByRole("textbox",{ name: label,exact: true });
  const commit = label => group(label).getByRole("button",{ name: "確定",exact: true }).click();
  const cancel = label => group(label).getByRole("button",{ name: "キャンセル",exact: true }).click();
  const row = at => editor.getByRole("article",{ name: `${at}の発言行`,exact: true });
  const input = at => row(at).getByRole("textbox",{ name: `${at}の発言`,exact: true });
  const edit = async (at = "00:00:00") => { await row(at).getByRole("button",{ name: `${at}の発言を編集`,exact: true }).click(); await input(at).waitFor(); };
  const bodyWrites = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft"));
  const recoveryCalls = command => page.evaluate(c => window.monBrowserFixture.recovery.calls.filter(x => x.command === c),command);
  const record = () => page.evaluate(() => { const f = window.monBrowserFixture; return f.recovery.records[f.draft.id]; });
  const savedName = value => page.waitForFunction(v => { const f = window.monBrowserFixture; return f.recovery.records[f.draft?.id]?.payload.nameBuffers?.some(b => b.value === v); },value);
  const savedText = value => page.waitForFunction(v => { const f = window.monBrowserFixture; return f.recovery.records[f.draft?.id]?.payload.buffers.some(b => b.value === v); },value);
  const openMeeting = async () => {
    await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    await page.getByRole("button",{ name: "開く",exact: true }).first().click(); await detailUI.openTranscriptTools(); await name("話者A").waitFor();
  };
  const boot = async () => { await page.reload(); await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click(); await name("話者A").waitFor(); };
  const crash = async () => { await page.evaluate(() => window.monBrowserFixture.crashSeed()); await page.reload(); await openMeeting(); };
  const dialog = () => page.getByRole("dialog",{ name: "未確定入力の回復記録",exact: true });
  const preview = async () => { await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).click(); await dialog().waitFor(); };
  const resume = async () => { await preview(); await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).click(); await dialog().waitFor({ state: "hidden" }); };
  const confirmDiscard = async () => { await dialog().getByRole("button",{ name: "この回復入力を破棄",exact: true }).click(); await page.getByRole("button",{ name: "確認した未確定入力を破棄する",exact: true }).click(); };

  await boot(); await name("話者A").fill("回復する未確定名"); await savedName("回復する未確定名");
  check((await record()).payload.schemaVersion === 3 && (await record()).payload.buffers.length === 0 && (await bodyWrites()).length === 0 && await page.evaluate(() => !window.monBrowserFixture.transcriptHistory.length && !window.monBrowserFixture.draft.payload.speaker_names.length),"name typing published canonical names or Undo");
  check(await editor.getByRole("checkbox",{ name: "田中",exact: true }).first().isVisible(),"unconfirmed name changed labels");
  results.push("name-only input is recovery format 3, distinct from confirmed labels, body and Undo");
  await crash(); await page.waitForTimeout(650);
  check(await name("話者A").inputValue() === "田中" && await name("話者A").isDisabled() && (await bodyWrites()).length === 0,"fresh reload automatically adopted the name");
  await preview();
  check(await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).inputValue() === "回復する未確定名" && await dialog().getByRole("textbox",{ name: "入力開始時の話者名",exact: true }).inputValue() === "田中" && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).evaluate(e => e.readOnly),"name-only preview lost baseline/value or is editable");
  await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check((await record()).payload.nameBuffers.length === 1 && (await recoveryCalls("discard_transcript_edit_recovery")).length === 0,"preview close erased names");
  results.push("fresh document reload reviews name-only input; selectable preview and close make no write");
  await resume(); await page.waitForTimeout(650);
  check(await name("話者A").inputValue() === "回復する未確定名" && await editor.getByRole("checkbox",{ name: "田中",exact: true }).first().isVisible() && (await bodyWrites()).length === 0,"resume confirmed a recovered name");
  results.push("explicit fresh-read name resume restores editing fields without changing confirmed speaker labels");
  await commit("話者A"); await group("話者A").getByRole("button",{ name: "確定",exact: true }).waitFor({ state: "hidden" });
  check((await record()).payload.schemaVersion === 1 && !(await record()).payload.nameBuffers && (await bodyWrites()).length === 1,"name confirmation failed to clear its journal");
  await crash(); check(await name("話者A").inputValue() === "回復する未確定名" && await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).count() === 0,"confirmed name resurrected as pending input");
  results.push("confirmed recovered name clears to legacy empty format and is not resurrected by a second reload");

  await boot(); await edit(); await input("00:00:00").fill("回復する本文"); await name("話者A").fill("本文と一緒の名前"); await name("話者B").fill("別の回復名"); await savedName("別の回復名"); await crash(); await preview();
  check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).count() === 1 && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).count() === 2 && await dialog().getByText(/入力 3件（本文 1件・話者名 2件）/).isVisible(),"mixed preview or counts omitted names");
  await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).click(); await dialog().waitFor({ state: "hidden" });
  check(await input("00:00:00").inputValue() === "回復する本文" && await name("話者A").inputValue() === "本文と一緒の名前" && await name("話者B").inputValue() === "別の回復名" && (await bodyWrites()).length === 0,"mixed resume applied only one input type");
  results.push("mixed review counts and restores both names and text atomically without canonical publishing");
  await commit("話者A"); await group("話者A").getByRole("button",{ name: "確定",exact: true }).waitFor({ state: "hidden" });
  check((await record()).baseDraftLockVersion === 1 && (await record()).payload.buffers[0].value === "回復する本文" && (await record()).payload.nameBuffers.length === 1 && (await record()).payload.nameBuffers[0].speakerId === "speaker-b" && await page.evaluate(() => !window.monBrowserFixture.draft.payload.utterances.length),"name confirmation erased or confirmed unrelated buffers");
  results.push("one name confirmation rebinds remaining text and names to its acknowledged draft");
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check((await record()).baseDraftLockVersion === 2 && !(await record()).payload.buffers.length && (await record()).payload.nameBuffers[0].value === "別の回復名","text confirmation cleared unconfirmed name");
  await cancel("話者B"); await group("話者B").getByRole("button",{ name: "キャンセル",exact: true }).waitFor({ state: "hidden" });
  check((await bodyWrites()).length === 2 && !(await record()).payload.nameBuffers && await name("話者B").inputValue() === "佐藤","name cancellation published another body");
  results.push("text confirmation preserves name recovery; cancelling an unpublished name clears it without another body write");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.recovery.lostSave = true; }); await name("話者A").fill("旧名の回復要求");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor(); await name("話者A").fill("最新の未確定名");
  const oldCount = (await recoveryCalls("save_transcript_edit_recovery")).length;
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click(); await page.waitForTimeout(650);
  check((await recoveryCalls("save_transcript_edit_recovery")).length === oldCount && await name("話者A").inputValue() === "最新の未確定名" && (await record()).payload.nameBuffers[0].value === "旧名の回復要求","read-back replaced or sent new name");
  await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedName("最新の未確定名");
  check((await bodyWrites()).length === 0,"name recovery retry published canonical names");
  results.push("lost named recovery read-back is read-only; latest name waits for explicit recovery retry");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = true; }); await name("話者A").fill("同じ旧要求を再試行");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor(); await name("話者A").fill("次の名前要求"); await edit(); await input("00:00:00").fill("次の本文要求");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click(); await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = false; });
  await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedName("次の名前要求");
  const retries = await recoveryCalls("save_transcript_edit_recovery");
  check(retries.length === 3 && JSON.stringify(retries[0].args) === JSON.stringify(retries[1].args) && retries[2].args.request.operationId !== retries[0].args.request.operationId && (await record()).payload.buffers[0].value === "次の本文要求" && (await bodyWrites()).length === 0,"named retry mutated old request or omitted new text");
  results.push("failed named request retries identical operation before a new mixed-input CAS");

  await boot(); await name("話者A").fill("取消済み名を復活させない"); await savedName("取消済み名を復活させない");
  await page.evaluate(() => { window.monBrowserFixture.recovery.lostClear = true; }); await cancel("話者A");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor();
  check(await name("話者A").inputValue() === "取消済み名を復活させない" && !(await record()).payload.nameBuffers,"unknown clear prematurely removed name field");
  await edit(); await input("00:00:00").fill("別欄の本文は保持"); await name("話者B").fill("別欄の名前は保持");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click(); await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedName("別欄の名前は保持");
  await group("話者A").getByRole("button",{ name: "キャンセル",exact: true }).waitFor({ state: "hidden" });
  check((await record()).payload.nameBuffers.length === 1 && (await record()).payload.nameBuffers[0].speakerId === "speaker-b" && (await record()).payload.buffers[0].value === "別欄の本文は保持" && (await bodyWrites()).length === 0,"other typing revived a cancelled name");
  results.push("lost name-clear acknowledgement retains controls; other text/name typing cannot revive consented removal");

  await boot(); await name("話者A").fill("古い除去応答"); await savedName("古い除去応答");
  await page.evaluate(() => { window.monBrowserFixture.recovery.saveWait = true; }); await cancel("話者A"); await page.waitForFunction(() => !!window.monBrowserFixture.recovery.releaseSave);
  await name("話者A").fill("除去待ち中の新しい名"); await name("話者B").fill("フォーカスを保持する名");
  await page.evaluate(() => { window.monBrowserFixture.recovery.releaseSave(); window.monBrowserFixture.recovery.releaseSave = null; }); await savedName("除去待ち中の新しい名");
  check(await name("話者A").inputValue() === "除去待ち中の新しい名" && await name("話者B").evaluate(e => document.activeElement === e) && (await bodyWrites()).length === 0,"old clear erased new input or stole focus");
  results.push("typing into a name during its old clear preserves the new session and another field's focus");

  await boot(); await name("話者A").fill("移動で確定する名"); await name("話者B").fill("移動で確定する別名"); await edit(); await input("00:00:00").fill("移動で確定する本文");
  await page.evaluate(async () => { await (await import("/src/draft-save.ts")).flushPendingEdits(); });
  check(!(await record()).payload.buffers.length && !(await record()).payload.nameBuffers && (await bodyWrites()).length === 1,"save barrier left confirmed names in recovery");
  await name("話者A").press("Escape"); await group("話者A").getByRole("button",{ name: "キャンセル",exact: true }).waitFor({ state: "hidden" });
  check(await name("話者A").inputValue() === "田中" && await page.evaluate(() => window.monBrowserFixture.draft.payload.speaker_names[0]?.display_name === "移動で確定する別名" && window.monBrowserFixture.draft.payload.utterances[0]?.effective_text === "移動で確定する本文") && (await bodyWrites()).length === 2,"barrier cancellation lost baseline or unrelated canonical text/name");
  results.push("navigation barrier clears both recovery arrays; later Escape restores only its starting name as a new version");

  await boot(); await name("話者A").fill("破棄確認する名だけ"); await savedName("破棄確認する名だけ"); await crash(); await preview();
  await dialog().getByRole("button",{ name: "この回復入力を破棄",exact: true }).click(); await page.getByRole("button",{ name: "確認を閉じる",exact: true }).click();
  check((await record()).payload.nameBuffers.length === 1 && !(await recoveryCalls("discard_transcript_edit_recovery")).length,"cancelled name discard erased input");
  await confirmDiscard(); await dialog().getByText("確認した未確定入力を破棄しました。確定本文と編集履歴は変更していません。",{ exact: true }).waitFor();
  check((await record()).payload.schemaVersion === 2 && !!(await record()).payload.discardedFrom && !(await record()).payload.buffers.length && !(await record()).payload.nameBuffers && (await bodyWrites()).length === 0,"name-only discard lacks exact proof");
  results.push("name-only discard requires confirmation and clears to an exact-proof tombstone without canonical changes");

  await boot(); await name("話者A").fill("破棄応答を失う名"); await savedName("破棄応答を失う名"); await crash(); await preview();
  await page.evaluate(() => { window.monBrowserFixture.recovery.lostDiscard = true; }); await confirmDiscard(); await page.getByRole("button",{ name: "確認を閉じる",exact: true }).click(); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  await editor.getByRole("button",{ name: "破棄結果を確認",exact: true }).click();
  check((await recoveryCalls("discard_transcript_edit_recovery")).length === 1 && await name("話者A").isEnabled() && (await bodyWrites()).length === 0,"lost name discard recheck wrote again or kept review blocked");
  results.push("lost name-discard result resolves by read-only proof without another deletion or publishing names");

  await boot(); await name("話者A").fill("古い版の名前"); await savedName("古い版の名前");
  await page.evaluate(() => { const d = window.monBrowserFixture.draft; d.lockVersion++; d.payload.speaker_names = [{ speaker_id: "speaker-a",display_name: "別操作の確定名" }]; }); await crash(); await preview();
  check(await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).isDisabled() && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).inputValue() === "古い版の名前","stale named record applied automatically");
  await confirmDiscard(); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check(await name("話者A").inputValue() === "別操作の確定名" && (await bodyWrites()).length === 0,"stale name discard overwrote newer canonical name");
  results.push("stale names remain read-only and discard preserves the newer confirmed name");

  await boot(); await name("話者A").fill("旧解析の名前"); await savedName("旧解析の名前");
  await page.evaluate(() => { const f = window.monBrowserFixture,old = structuredClone(f.draft); f.archivedDrafts[old.baseAnalysisId] = old; f.draft = { ...old,id: crypto.randomUUID(),baseAnalysisId: f.candidate.id,lockVersion: 0,payload: { schema_version: 1,speaker_names: [],utterances: [] } }; f.currentId = f.candidate.id; }); await crash();
  await editor.getByRole("button",{ name: "旧下書きを含む回復記録",exact: true }).click(); await dialog().getByRole("button",{ name: "この記録を見る",exact: true }).click();
  check(await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).isDisabled() && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).inputValue() === "旧解析の名前","archived name cannot be read or was resumable");
  const owner = await page.evaluate(() => window.monBrowserFixture.draft.id); await confirmDiscard();
  check(await page.evaluate(id => window.monBrowserFixture.draft.id === id && window.monBrowserFixture.currentId === window.monBrowserFixture.candidate.id,owner) && (await bodyWrites()).length === 0,"archived name discard adopted old analysis");
  results.push("inventory counts archived names and keeps them copyable; discard never adopts their draft or analysis");

  await boot(); await name("話者A").fill("見えない統合元を保持"); await savedName("見えない統合元を保持");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture,r = f.recovery.records[f.draft.id],module = await import("/tests/transcript-recovery-browser-fixture.js");
    f.draft.payload.speaker_merges = [{ source_id: "speaker-a",target_id: "speaker-b" }]; await module.stampFixtureDraft(f.draft);
    r.payload.basePayload = structuredClone(f.draft.payload); r.baseDraftContentHash = f.draft.contentHash; r.contentHash = await module.fixtureHash(r.payload);
  });
  await page.evaluate(() => window.monBrowserFixture.crashSeed()); await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled); await page.getByRole("button",{ name: "開く",exact: true }).first().click(); await detailUI.openTranscriptTools(); await name("話者B").waitFor();
  await preview(); await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).click(); await dialog().getByRole("alert").filter({ hasText: "部分的に再開せず" }).waitFor();
  check((await record()).payload.nameBuffers[0].value === "見えない統合元を保持" && await name("話者B").isDisabled() && (await bodyWrites()).length === 0,"invisible recovered name was silently applied or lost");
  results.push("merged-away speaker name refuses partial resume while retaining the complete copyable record");

  await boot(); await edit(); await input("00:00:00").fill("再読込で破棄する本文"); await name("話者A").fill("再読込で破棄する名前"); await savedName("再読込で破棄する名前");
  await editor.getByRole("button",{ name: "再読込",exact: true }).click(); await page.getByRole("button",{ name: "入力を破棄して再読込する",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check(!(await record()).payload.buffers.length && !(await record()).payload.nameBuffers && await name("話者A").inputValue() === "田中" && (await bodyWrites()).length === 0,"local reload discarded only one input type");
  await crash(); check(await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).count() === 0,"consented mixed discard resurrected input");
  results.push("consented local reload discards both input types before replacement and neither revives on document reload");

  await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({ length: 9 },(_,i) => ({ id: `speech-${i}`,trackId: "microphone",startUs: i * 1_000_000,endUs: (i + 1) * 1_000_000,recognizedText: `原文${i}`,effectiveText: `原文${i}`,attributionState: "assigned",speakerIds: ["speaker-a"] }))));
  await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click();
  for(let i = 0; i < 9; i++) { const at = `00:00:${String(i).padStart(2,"0")}`; await edit(at); await input(at).fill(`回復本文${i}`); }
  await name("話者A").fill("一ページ目の回復名"); await name("話者B").fill("二ページ目の回復名"); await savedName("二ページ目の回復名"); await savedText("回復本文8"); await crash(); await preview();
  check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).count() === 9 && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).count() === 1,"mixed preview limit counts only text");
  await dialog().getByRole("button",{ name: "次の入力",exact: true }).click();
  check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).count() === 0 && await dialog().getByRole("textbox",{ name: "未確定の話者名",exact: true }).inputValue() === "二ページ目の回復名","mixed pagination lost trailing name");
  results.push("mixed preview pages at ten total inputs and preserves names on the last page");
  await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  for(const theme of ["light","dark"]) {
    await page.getByRole("banner").getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption(theme); await preview();
    await dialog().getByRole("button",{ name: "次の入力",exact: true }).click();
    await page.setViewportSize({ width: theme === "light" ? 1200 : 960,height: 900 }); await page.waitForTimeout(150);
    check(await dialog().evaluate(e => { const r = e.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth && e.scrollWidth <= e.clientWidth + 1 && [...e.querySelectorAll("textarea")].every(t => t.readOnly); }),"name recovery preview overflow or writable field");
    await page.screenshot({ path: `speaker-name-recovery-${theme}.png` }); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  }
  results.push("light/dark named recovery preview keeps selectable read-only fields and contained layout");
  return { passed: results.length,results };
}

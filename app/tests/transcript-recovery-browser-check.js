// Real production React and browser input; all backend/storage data is synthetic.
async (page, capturePath, detailUI) => {
  const results = [],check = (ok,message) => { if(!ok) throw Error(message); };
  const editor = page.getByRole("region",{ name: "文字起こし編集",exact: true });
  const row = at => editor.getByRole("article",{ name: `${at}の発言行`,exact: true });
  const input = at => row(at).getByRole("textbox",{ name: `${at}の発言`,exact: true });
  const read = at => row(at).getByLabel(`${at}の発言本文`,{ exact: true });
  const edit = async (at = "00:00:00") => { await row(at).getByRole("button",{ name: `${at}の発言を編集`,exact: true }).click(); await input(at).waitFor(); };
  const bodyWrites = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft"));
  const recoveryCalls = command => page.evaluate(c => window.monBrowserFixture.recovery.calls.filter(x => x.command === c),command);
  const record = () => page.evaluate(() => { const f = window.monBrowserFixture; return f.recovery.records[f.draft.id]; });
  const savedInput = value => page.waitForFunction(v => { const f = window.monBrowserFixture; return f.recovery.records[f.draft?.id]?.payload.buffers.some(b => b.value === v); },value);
  const openMeeting = async () => {
    await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    await page.getByRole("button",{ name: "開く",exact: true }).first().click(); await detailUI.openTranscriptTools(); await read("00:00:00").waitFor();
  };
  const boot = async () => { await page.reload(); await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click(); await edit(); };
  const crash = async () => { await page.evaluate(() => window.monBrowserFixture.crashSeed()); await page.reload(); await openMeeting(); };
  const dialog = () => page.getByRole("dialog",{ name: "未確定入力の回復記録",exact: true });
  const preview = async () => { await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).click(); await dialog().waitFor(); };
  const resume = async () => { await preview(); await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).click(); await dialog().waitFor({ state: "hidden" }); };
  const confirmDiscard = async () => { await dialog().getByRole("button",{ name: "この回復入力を破棄",exact: true }).click(); await page.getByRole("button",{ name: "確認した未確定入力を破棄する",exact: true }).click(); };

  await boot(); await input("00:00:00").fill("再起動後に回復する未確定本文"); await savedInput("再起動後に回復する未確定本文");
  check((await bodyWrites()).length === 0 && await page.evaluate(() => window.monBrowserFixture.transcriptHistory.length === 0 && window.monBrowserFixture.draft.payload.utterances.length === 0),"typing published body or Undo");
  check(await editor.getByText("未保存",{ exact: true }).isVisible() && await input("00:00:00").getAttribute("maxlength") === "20000","recovery save falsely reported confirmed body or lacks input limit");
  results.push("idle typing durably saves only recovery input; body, Undo and unsaved status remain distinct");
  await crash(); await page.waitForTimeout(650);
  check(await input("00:00:00").count() === 0 && await editor.getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).isDisabled() && (await bodyWrites()).length === 0,"crash automatically applied recovery");
  results.push("fresh document reload discovers recovery without applying input or publishing body");
  await preview(); check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).inputValue() === "再起動後に回復する未確定本文","preview lost saved text");
  await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check((await record()).payload.buffers.length === 1 && (await recoveryCalls("discard_transcript_edit_recovery")).length === 0,"closing review discarded input");
  results.push("preview is selectable read-only and closing it leaves durable input unchanged");
  await resume(); check(await input("00:00:00").inputValue() === "再起動後に回復する未確定本文" && (await bodyWrites()).length === 0,"explicit resume confirmed text");
  results.push("fresh-read explicit resume opens the saved buffer but does not confirm body");
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check((await record()).payload.buffers.length === 0 && (await bodyWrites()).length === 1,"confirm did not clear recovery after body acknowledgement");
  await crash(); check(await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).count() === 0 && await read("00:00:00").textContent() === "再起動後に回復する未確定本文","confirmed input resurrected on reload");
  results.push("confirmation clears the journal after body CAS and a second restart cannot resurrect it");

  await boot(); await edit("00:00:20"); await input("00:00:00").fill("第一発言を確定"); await input("00:00:20").fill("第二発言は回復対象"); await savedInput("第二発言は回復対象");
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check((await record()).payload.buffers.length === 1 && (await record()).payload.buffers[0].value === "第二発言は回復対象" && (await record()).baseDraftLockVersion === 1,"confirm cleared unrelated buffer or failed to rebase");
  results.push("one-row confirmation rebinds and keeps other unconfirmed buffers on the acknowledged draft");
  await row("00:00:20").getByRole("button",{ name: "キャンセル",exact: true }).click(); await input("00:00:20").waitFor({ state: "hidden" });
  check((await record()).payload.buffers.length === 0 && (await bodyWrites()).length === 1,"cancel created unwanted publication");
  results.push("cancelling an unpublished buffer removes only recovery input and adds no body write");

  await boot(); await input("00:00:00").fill("旧要求を保持"); await page.evaluate(() => { window.monBrowserFixture.recovery.lostSave = true; });
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor();
  await input("00:00:00").fill("応答喪失後の新入力"); const countBeforeCheck = (await recoveryCalls("save_transcript_edit_recovery")).length;
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click(); await page.waitForTimeout(650);
  check((await recoveryCalls("save_transcript_edit_recovery")).length === countBeforeCheck && await input("00:00:00").inputValue() === "応答喪失後の新入力" && (await record()).payload.buffers[0].value === "旧要求を保持","read-only result check sent latest input or lost text");
  results.push("lost recovery ack check is read-only; newer local input is retained without automatic resend");
  await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedInput("応答喪失後の新入力");
  check((await bodyWrites()).length === 0,"recovery retry published body");
  results.push("explicit recovery retry saves latest retained input only after resolving the old result");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = true; }); await input("00:00:00").fill("未保存の旧要求");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor(); await input("00:00:00").fill("未保存後の最新入力");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click();
  await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = false; }); await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedInput("未保存後の最新入力");
  const retries = await recoveryCalls("save_transcript_edit_recovery");
  check(retries.length === 3 && JSON.stringify(retries[0].args) === JSON.stringify(retries[1].args) && retries[2].args.request.operationId !== retries[0].args.request.operationId,"retry replaced original operation");
  results.push("unchanged recovery head retains the exact old request; explicit retry resolves it before a new CAS");

  await boot(); await input("00:00:00").fill("本文応答喪失時の確定要求"); await page.evaluate(() => { window.monBrowserFixture.lostTranscriptSaveResponse = true; });
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).waitFor();
  await input("00:00:00").fill("本文応答後も保持する新入力"); const beforeBodyCheck = (await bodyWrites()).length;
  await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).click();
  check((await bodyWrites()).length === beforeBodyCheck && await input("00:00:00").inputValue() === "本文応答後も保持する新入力","body read-back published new text");
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check((await bodyWrites()).length === 2 && await read("00:00:00").textContent() === "本文応答後も保持する新入力","new input could not be confirmed after read-back");
  results.push("lost body result check avoids duplicate publishing and lets newer retained text be explicitly confirmed");

  await boot(); await input("00:00:00").fill("本文未保存の先行要求"); await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).waitFor();
  await input("00:00:00").fill("読戻し後も明示確定が必要");
  const unchangedBodyCount = (await bodyWrites()).length,unchangedJournalCount = (await recoveryCalls("save_transcript_edit_recovery")).length;
  await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).click(); await page.waitForTimeout(900);
  check((await bodyWrites()).length === unchangedBodyCount && (await recoveryCalls("save_transcript_edit_recovery")).length === unchangedJournalCount && await input("00:00:00").inputValue() === "読戻し後も明示確定が必要","unchanged body result check automatically resent input");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; }); await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check(await read("00:00:00").textContent() === "読戻し後も明示確定が必要","retained latest input could not be explicitly confirmed");
  results.push("unchanged body read-back pauses both autosave timers until explicit confirmation or retry");

  await boot(); await edit("00:00:20"); await input("00:00:00").fill("回復除去の応答待ち"); await savedInput("回復除去の応答待ち");
  await page.evaluate(() => { window.monBrowserFixture.recovery.lostClear = true; });
  await row("00:00:00").getByRole("button",{ name: "確定",exact: true }).click();
  // Other open field remains present in the clear request, so use a single
  // committed row for the empty tombstone lost-ack path.
  await input("00:00:00").waitFor({ state: "hidden" });
  await row("00:00:20").getByRole("button",{ name: "キャンセル",exact: true }).click();
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor();
  check(await input("00:00:20").isVisible() && (await record()).payload.buffers.length === 0,"unknown clear removed editor before acknowledgement");
  await edit("00:00:00"); await input("00:00:00").fill("別欄の新入力");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click();
  await editor.getByRole("button",{ name: "未確定入力の保存を再試行",exact: true }).click(); await savedInput("別欄の新入力"); await input("00:00:20").waitFor({ state: "hidden" });
  check((await record()).payload.buffers.length === 1 && (await record()).payload.buffers[0].utteranceId === "speech-a","other typing resurrected cancelled recovery buffer");
  results.push("lost clear ack keeps the old field visible; other typing cannot resurrect its consented removal");

  await boot(); await input("00:00:00").fill("取消済み本文を再保存しない"); await savedInput("取消済み本文を再保存しない");
  await page.evaluate(() => { window.monBrowserFixture.recovery.lostClear = true; }); await row("00:00:00").getByRole("button",{ name: "キャンセル",exact: true }).click();
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor();
  await editor.getByRole("textbox",{ name: "話者A",exact: true }).fill("名前だけ変更");
  await editor.getByRole("group",{ name: "話者Aの名前編集",exact: true }).getByRole("button",{ name: "確定",exact: true }).click();
  await editor.getByRole("button",{ name: "保存を再試行",exact: true }).waitFor();
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  await editor.getByRole("button",{ name: "保存を再試行",exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.speaker_names[0]?.display_name === "名前だけ変更");
  check(await page.evaluate(() => window.monBrowserFixture.draft.payload.utterances.length === 0) && (await bodyWrites()).length === 1,"body retry resurrected the cancelled buffer");
  results.push("a body retry after unknown recovery clear saves unrelated names without republishing cancelled input");

  await boot(); await input("00:00:00").fill("明示破棄する回復入力"); await savedInput("明示破棄する回復入力"); await crash(); await preview();
  await dialog().getByRole("button",{ name: "この回復入力を破棄",exact: true }).click(); await page.getByRole("button",{ name: "確認を閉じる",exact: true }).click();
  check((await record()).payload.buffers.length === 1 && (await recoveryCalls("discard_transcript_edit_recovery")).length === 0,"cancelled confirmation discarded journal");
  results.push("discard confirmation defaults to cancel and cancellation preserves the exact preview");
  await confirmDiscard(); await dialog().getByText("確認した未確定入力を破棄しました。確定本文と編集履歴は変更していません。",{ exact: true }).waitFor();
  const cleared = await record(); check(cleared.payload.schemaVersion === 2 && cleared.payload.discardedFrom.lockVersion === 1 && cleared.payload.buffers.length === 0 && (await bodyWrites()).length === 0,"discard lacks exact proof or published body");
  await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check(await editor.getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).isEnabled(),"discard did not release current review");
  results.push("explicit reviewed discard creates an exact-proof tombstone and leaves confirmed body/Undo untouched");

  await boot(); await input("00:00:00").fill("破棄応答不明でも確認可能"); await savedInput("破棄応答不明でも確認可能"); await crash(); await preview();
  await page.evaluate(() => { window.monBrowserFixture.recovery.lostDiscard = true; }); await confirmDiscard();
  await page.getByRole("button",{ name: "確認を閉じる",exact: true }).click(); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  await editor.getByRole("button",{ name: "破棄結果を確認",exact: true }).waitFor();
  await page.getByRole("navigation",{ name: "メインナビゲーション",exact: true }).getByRole("button",{ name: "設定",exact: true }).click();
  check(await editor.isVisible(),"unknown discard allowed navigation and lost request");
  await editor.getByRole("button",{ name: "破棄結果を確認",exact: true }).click();
  check((await recoveryCalls("discard_transcript_edit_recovery")).length === 1 && await editor.getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).isEnabled(),"lost discard check wrote again or failed to release review");
  results.push("closing failed discard preserves its request and blocks navigation; proof read-back makes no new deletion");

  await boot(); await input("00:00:00").fill("古い本文の回復記録"); await savedInput("古い本文の回復記録");
  await page.evaluate(() => { const d = window.monBrowserFixture.draft; d.lockVersion++; d.payload = { ...d.payload,speaker_names: [{ speaker_id: "speaker-a",display_name: "本文更新済み" }] }; }); await crash(); await preview();
  check(await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).isDisabled() && await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).inputValue() === "古い本文の回復記録","stale journal was resumable or unreadable");
  results.push("a newer body stamp makes old input read-only/manual-copy, not resumable");
  await confirmDiscard(); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check(await editor.getByRole("textbox",{ name: "話者A",exact: false }).inputValue() === "本文更新済み" && (await bodyWrites()).length === 0,"stale discard overwrote current speaker name");
  results.push("stale preview discard uses the current body stamp and preserves newer names/body");

  await boot(); await input("00:00:00").fill("旧解析に残る入力"); await savedInput("旧解析に残る入力");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture,old = structuredClone(f.draft); f.archivedDrafts[old.baseAnalysisId] = old;
    f.draft = { ...structuredClone(old),id: crypto.randomUUID(),baseAnalysisId: f.candidate.id,lockVersion: 0,payload: { schema_version: 1,speaker_names: [],utterances: [] } }; f.currentId = f.candidate.id;
    const r = structuredClone(f.recovery.records[old.id]);
    for(let i = 0; i < 24; i++) { const d = { ...structuredClone(old),id: `00000000-0000-4000-8000-${String(i).padStart(12,"0")}`,baseAnalysisId: `old-${i}` }; f.archivedDrafts[d.baseAnalysisId] = d; f.recovery.records[d.id] = { ...structuredClone(r),draftId: d.id,baseAnalysisId: d.baseAnalysisId }; }
  }); await crash();
  await editor.getByRole("button",{ name: "旧下書きを含む回復記録",exact: true }).click();
  await dialog().getByText("一覧 1ページ · 20件（最大20件）",{ exact: true }).waitFor();
  await dialog().getByRole("button",{ name: "次の一覧",exact: true }).click(); await dialog().getByText("一覧 2ページ · 5件（最大20件）",{ exact: true }).waitFor();
  await dialog().getByRole("button",{ name: "この記録を見る",exact: true }).last().click();
  check(await dialog().getByRole("button",{ name: "確認した入力の編集を再開",exact: true }).isDisabled() && await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).inputValue() === "旧解析に残る入力","inactive journal applied over current analysis");
  results.push("meeting-scoped recovery inventory pages 20 at a time and shows inactive input without adopting it");
  const currentBefore = await page.evaluate(() => ({ id: window.monBrowserFixture.draft.id,currentId: window.monBrowserFixture.currentId }));
  await confirmDiscard(); await dialog().getByRole("button",{ name: "閉じる",exact: true }).click();
  check(JSON.stringify(currentBefore) === JSON.stringify(await page.evaluate(() => ({ id: window.monBrowserFixture.draft.id,currentId: window.monBrowserFixture.currentId }))) && (await bodyWrites()).length === 0,"old discard activated or published old draft");
  results.push("discarding inactive recovery cannot activate its draft, adopt analysis or create Undo");

  await boot(); await input("00:00:00").fill("確認後の変更は破棄しない"); await savedInput("確認後の変更は破棄しない"); await crash(); await preview();
  await page.evaluate(async () => { const f = window.monBrowserFixture,r = f.recovery.records[f.draft.id]; r.payload.buffers[0].value = "別操作で更新した入力"; r.contentHash = await (await import("/tests/transcript-recovery-browser-fixture.js")).fixtureHash(r.payload); r.lockVersion++; r.operationId = crypto.randomUUID(); });
  await confirmDiscard(); await page.getByRole("button",{ name: "確認を閉じる",exact: true }).waitFor();
  check((await record()).payload.buffers[0].value === "別操作で更新した入力","stale consent discarded newer input");
  results.push("changed preview is rejected by exact journal CAS and newer input stays intact");

  await boot(); await input("00:00:00").fill("破棄再読込で復活しない"); await savedInput("破棄再読込で復活しない");
  await editor.getByRole("button",{ name: "再読込",exact: true }).click(); await page.getByRole("button",{ name: "入力を破棄して再読込する",exact: true }).click(); await input("00:00:00").waitFor({ state: "hidden" });
  check((await record()).payload.buffers.length === 0 && (await bodyWrites()).length === 0,"explicit reload discard retained recoverable input or published body");
  await crash(); check(await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).count() === 0,"discarded reload input reappeared");
  results.push("consented local reload clears recovery before replacing the UI and cannot resurrect input after restart");

  await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({ length: 12 },(_,i) => ({ id: `speech-${i}`,trackId: "microphone",startUs: i * 1_000_000,endUs: (i + 1) * 1_000_000,recognizedText: `原文${i}`,effectiveText: `原文${i}`,attributionState: "assigned",speakerIds: ["speaker-a"] }))));
  await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click();
  for(let i = 0; i < 12; i++) { const at = `00:00:${String(i).padStart(2,"0")}`; await edit(at); await input(at).fill(`回復入力${i}`); }
  await savedInput("回復入力11"); await crash(); await preview();
  check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).count() === 10,"preview is not bounded to ten inputs");
  await dialog().getByRole("button",{ name: "次の入力",exact: true }).click();
  check(await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).count() === 2 && await dialog().getByRole("textbox",{ name: "未確定の本文",exact: true }).allTextContents().then(values => values.includes("回復入力9")),"last preview page lost an input");
  results.push("twelve saved input buffers remain available across ten-entry preview pages without publishing body");

  await boot(); await input("00:00:00").fill("回復確認の見た目"); await savedInput("回復確認の見た目"); await crash();
  await page.getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption("light"); await page.waitForFunction(() => document.documentElement.dataset.theme === "light"); await preview();
  await page.screenshot({ path: "output/playwright/transcript-recovery-light.png" });
  await dialog().getByRole("button",{ name: "閉じる",exact: true }).click(); await page.getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption("dark"); await page.waitForFunction(() => document.documentElement.dataset.theme === "dark"); await preview();
  await page.setViewportSize({ width: 960,height: 720 }); await page.waitForTimeout(150); await page.screenshot({ path: "output/playwright/transcript-recovery-dark.png" });
  check(await dialog().evaluate(e => { const r = e.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth && e.scrollWidth <= e.clientWidth + 1; }),"recovery dialog overflows 960px");
  results.push("read-only recovery preview renders within 960px and retains input across light/dark themes");
  await page.setViewportSize({ width: 360,height: 640 });
  await page.waitForTimeout(150);
  check(await dialog().evaluate(e => { const r = e.getBoundingClientRect(); return r.left >= 24 && r.right <= innerWidth - 24 && e.scrollWidth <= e.clientWidth + 1 && [...e.querySelectorAll("textarea")].every(t => t.readOnly && t.getBoundingClientRect().width <= e.clientWidth); }),"narrow preview clips text or fields");
  await page.screenshot({ path: "output/playwright/transcript-recovery-dark-360.png" });
  results.push("360px review keeps 24px outer margins and full-width selectable read-only fields without clipping");
  return results;
}

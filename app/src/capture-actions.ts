import { useEffect, useRef, useState, type Dispatch, type RefObject, type SetStateAction } from "react";
import { invoke } from "./ipc.ts";
import { toasts } from "./toast";
import type { useCaptureInputs } from "./capture-inputs";
import type { useCaptureScope } from "./capture-applications";
import type { useCaptureTest } from "./capture-test";
import type { useRecordingSession, RecordingSnapshot } from "./session";
import type { useRecordingConfirmation } from "./recording-confirmation";
import type { useRecordingMicrophoneSelection } from "./microphone-selection-status";
import type { useActionConfirmation } from "./confirmation-service";
import type { useSavedNavigation } from "./saved-navigation";
import type { ImportSnapshot } from "./import-state";
import { withConfirmedEdits } from "./confirmed-action";
import { stopAllPlayback } from "./playback-barrier";
import { newRecordingPreparation, sameRecordingPreparation, validRecordingTitle, type RecordingPreparation } from "./recording-preparation-state";
import { checkedCaptureInputs, usableAutomaticMicrophone, usableMicrophone } from "./capture-input-state";
import { captureApplicationScope, checkedCaptureApplications, checkedCaptureScope } from "./capture-scope-state";
import { assertCloseImport, assertCloseRecording, importNeedsConsent, recordingActive } from "./application-close";

export interface CaptureMeeting {
  id: string;
  title: string;
  lifecycle: string;
  sourceType?: "recording" | "audio_import" | null;
  createdAt: number;
  updatedAt: number;
  lockVersion: number;
}

/** State is shared with the view's protection rules before action dependencies
 * are composed; command ownership and acknowledgement remain in this module. */
export function useCaptureActionState() {
  const starting = useRef(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [captureError, setCaptureError] = useState("");
  const [captureTestError, setCaptureTestError] = useState("");
  const [microphoneRequest, setMicrophoneRequest] = useState<{ owner: string; revision: number } | null>(null);
  return { starting, captureBusy, setCaptureBusy, captureError, setCaptureError, captureTestError, setCaptureTestError, microphoneRequest, setMicrophoneRequest };
}
interface CaptureContext {
  selectedMeeting: CaptureMeeting | null;
  preparation: RecordingPreparation | null;
  screen: string;
  baseWriteLocked: boolean;
  captureAvailable: boolean;
  captureInputs: ReturnType<typeof useCaptureInputs>;
  captureScope: ReturnType<typeof useCaptureScope>;
  captureTest: ReturnType<typeof useCaptureTest>;
  state: ReturnType<typeof useRecordingSession>;
}
interface CaptureDependencies {
  context: CaptureContext;
  controller: ReturnType<typeof useCaptureActionState>;
  captureOwner: string | null;
  closing: RefObject<boolean>;
  activeMicrophoneInput: boolean;
  microphoneSession: string | null;
  microphoneSelection: ReturnType<typeof useRecordingMicrophoneSelection>;
  startConfirm: ReturnType<typeof useActionConfirmation>;
  withSavedEdits: ReturnType<typeof useSavedNavigation>["withSavedEdits"];
  recordingDecision: Pick<ReturnType<typeof useRecordingConfirmation>, "request">;
  navigationLocked: boolean;
  setPreparation: Dispatch<SetStateAction<RecordingPreparation | null>>;
  setSelectedMeeting: (meeting: CaptureMeeting | null) => void;
  setRenameTitle: (title: string) => void;
  setScreen: (screen: "recording") => void;
  setMeetings: (meetings: CaptureMeeting[]) => void;
}
/** Recording/test confirmation, edit/playback barriers and native command
 * acknowledgement have one owner. Live context revalidates delayed consent;
 * uncertain starts are never resent. The view composes presentation only. */
export function useCaptureActions(dependencies: CaptureDependencies) {
  const { context, controller, captureOwner, closing, activeMicrophoneInput, microphoneSession, microphoneSelection,
    startConfirm, withSavedEdits, recordingDecision, navigationLocked, setPreparation, setSelectedMeeting, setRenameTitle, setScreen, setMeetings } = dependencies;
  const { selectedMeeting, preparation, screen, captureInputs, captureScope, captureTest, state } = context;
  const { starting, captureBusy, setCaptureBusy, setCaptureError, setCaptureTestError, microphoneRequest, setMicrophoneRequest } = controller;
  const root = useRef(context);
  root.current = context;
  useEffect(()=>{
    if(microphoneRequest&&(microphoneRequest.owner!==microphoneSession||microphoneSelection?.requestRevision!==undefined&&microphoneSelection.requestRevision>=microphoneRequest.revision))setMicrophoneRequest(null);
  },[microphoneSession,microphoneRequest,microphoneSelection?.requestRevision]);
  useEffect(()=>{
    if(!activeMicrophoneInput||!microphoneSelection||!captureInputs.inputs||microphoneRequest?.owner===microphoneSession&&microphoneSelection.requestRevision<microphoneRequest.revision)return;
    if(microphoneSelection.mode==="automatic"&&captureInputs.mode!=="automatic")captureInputs.select("");
    if(microphoneSelection.mode==="fixed"&&microphoneSelection.source!=="pending"&&captureInputs.inputs.devices.some(device=>device.uid===microphoneSelection.deviceUID)&&(captureInputs.mode!=="fixed"||captureInputs.uid!==microphoneSelection.deviceUID))captureInputs.select(microphoneSelection.deviceUID);
  },[activeMicrophoneInput,microphoneSelection?.revision,captureInputs.inputs,captureInputs.mode,captureInputs.uid,microphoneRequest,microphoneSession]);
  const startRecording = async () => {
    const local=screen==="recording"?preparation:null;
    if ((!selectedMeeting&&!local) || local&&(local.submitted||!validRecordingTitle(local.title)) || !captureInputs.selected || !captureScope.scope || starting.current || closing.current || root.current.baseWriteLocked) return;
    const owner=local?.id??selectedMeeting!.id;
    const microphoneMode=captureInputs.mode;
    const microphoneUID=microphoneMode==="fixed"?captureInputs.uid:null;
    const scope=checkedCaptureScope(captureScope.scope,captureScope.applications);
    const scopeKey=JSON.stringify(scope);
    const scopeUnchanged=()=>JSON.stringify(root.current.captureScope.scope)===scopeKey;
    starting.current = true; setCaptureBusy(true); setCaptureError("");
    try {
      const saved = await invoke<CaptureMeeting | null>("get_meeting", { id: owner });
      if(local&&saved)throw Error("録音準備の会議は既に保存されています。受付結果を確認してください。");
      const expected:CaptureMeeting|null=local?{id:local.id,title:local.title.trim(),lifecycle:"draft",sourceType:null,lockVersion:0,createdAt:0,updatedAt:0}:saved;
      const ownerUnchanged=()=>local?sameRecordingPreparation(root.current.preparation,local)&&root.current.screen==="recording":root.current.selectedMeeting?.id===owner;
      const importing = await invoke<ImportSnapshot>("get_audio_import"); assertCloseImport(importing);
      const expectedTest=await captureTest.client.refresh();
      if(expectedTest?.busy)throw Error("録音前テストの停止・検証が終わってから録音を開始してください。");
      const currentInputList=checkedCaptureInputs(await invoke("get_capture_inputs"));
      const microphone=microphoneUID===null?usableAutomaticMicrophone(currentInputList):usableMicrophone(currentInputList,microphoneUID);
      if(!microphone)throw Error("マイクまたは権限を再確認してください。録音は開始していません。");
      const applications=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,applications);
      const application=captureApplicationScope(scope,applications);
      const countsMessage=local?`\n録音後の会議全体の話者数: ${local.numSpeakers===null?"自動":"最大"+local.numSpeakers+"人"}。`:"";
      const scopeMessage=scope.mode==="system"?"PC音声は全体取得し、通知音や他の動画の音も含まれる場合があります。":`PC音声は選択したアプリ（${application?.name}）を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。終了・再起動時は全体取得や別アプリに切り替えません。`;
      if (!expected || expected.lifecycle !== "draft" || expected.sourceType != null || !ownerUnchanged()) throw Error("録音対象の会議を再確認してください。");
      await startConfirm({ title: "選択した会議の録音を開始しますか？", target: expected.title, confirmLabel: "録音を開始する", message: "マイクとPCの再生音が端末内に保存されます。参加者の同意と録音権限を確認してください。実行中の解析は安全な区切りで一時停止し、録音終了後に再開します。"+`\n録音するマイク: ${microphoneUID===null?"自動（会議アプリの使用マイクを優先し、不明なときはOSの既定入力）":microphone.name}。${scopeMessage}` + countsMessage + (expectedTest ? "\n前のテスト音声の再生を停止し、一時音声を破棄してから録音を開始します。保存済み会議の音声は変更しません。" : "") + (importNeedsConsent(importing) ? `\n取り込み（${importing.fileName ?? "選択・確認中の音声"}）を安全に取り消して録音を優先します。取り込みの確認内容と未確定物だけを整理し、元ファイルと保存済み会議を保持します。` : ""), validate: () => ownerUnchanged() && root.current.captureAvailable && root.current.captureInputs.mode===microphoneMode&&(microphoneUID===null||root.current.captureInputs.uid===microphoneUID)&&!!root.current.captureInputs.selected && scopeUnchanged() && !root.current.baseWriteLocked && root.current.state.kind === "ready" && ["idle", "error"].includes(root.current.state.snapshot.phase), action: () => withConfirmedEdits(withSavedEdits, async () => {
        const [meeting, recording, currentImport, currentInputs, currentApplications] = await Promise.all([invoke<CaptureMeeting | null>("get_meeting", { id: expected.id }), invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null)]);
        assertCloseRecording(recording);
        assertCloseImport(currentImport);
        if(!(microphoneUID===null?usableAutomaticMicrophone(currentInputs):usableMicrophone(currentInputs,microphoneUID))||root.current.captureInputs.mode!==microphoneMode||microphoneUID!==null&&root.current.captureInputs.uid!==microphoneUID)throw Error("確認したマイクまたは権限が変更されました。録音は開始していません。");
        checkedCaptureScope(scope,currentApplications);
        if(!scopeUnchanged())throw Error("PC音声の取得範囲が変更されました。録音は開始していません。");
        const meetingUnchanged=local?meeting===null:!!meeting&&meeting.id===expected.id&&meeting.title===expected.title&&meeting.lockVersion===expected.lockVersion&&meeting.lifecycle==="draft"&&meeting.sourceType==null;
        if (!meetingUnchanged || !ownerUnchanged() || recordingActive(recording) || currentImport.token !== importing.token || !importing.busy && currentImport.revision !== importing.revision) throw Error("録音対象が確認後に変更されました。録音は開始していません。");
        await stopAllPlayback();
        const currentTest=await captureTest.client.refresh();
        if((currentTest?.runId??null)!==(expectedTest?.runId??null)||currentTest?.busy)throw Error("録音前テストの対象・状態が変更されました。録音は開始していません。");
        if(currentTest)await captureTest.client.discard(currentTest.runId);
        if(local)setPreparation(current=>current?.id===local.id?{...current,submitted:true}:current);
        await invoke("start_recording", { meetingId: expected.id, microphoneUid:microphoneUID, captureScope:scope, newTitle:local?expected.title:null, numSpeakers:local?.numSpeakers??null });
        const accepted=await invoke<CaptureMeeting | null>("get_meeting", { id: expected.id });
        if(!accepted||accepted.id!==expected.id)throw Error("録音の会議保存結果を確認できません。開始は再送していません。");
        setSelectedMeeting(accepted);setRenameTitle(accepted.title);setPreparation(null);setScreen("recording");
        setMeetings(await invoke<CaptureMeeting[]>("list_meetings"));
      }), onError: cause => setCaptureError(`録音の受付状態を確認してください。開始は自動再送していません: ${String(cause)}`) });
    } catch (error) {
      setCaptureError(String(error));
    } finally {
      starting.current = false;
      setCaptureBusy(false);
    }
  };
  const selectRecordingMicrophone=async(value:string)=>{
    const snapshot=state.kind==="ready"?state.snapshot:null;
    const sessionId=snapshot?.sessionId;
    if(!sessionId||!["preparing","recording"].includes(snapshot.phase)||captureBusy||!captureInputs.inputs)return;
    if(value!==""&&!captureInputs.inputs.devices.some(device=>device.uid===value))return;
    setCaptureBusy(true);
    try{
      const current=await invoke<RecordingSnapshot>("get_recording_snapshot");
      if(current.sessionId!==sessionId||!["preparing","recording"].includes(current.phase))throw Error("録音が切り替わったか、停止しました。");
      const revision=await invoke<number>("select_recording_microphone",{sessionId,microphoneUid:value===""?null:value});
      setMicrophoneRequest({owner:sessionId,revision});
      if(root.current.state.kind==="ready"&&root.current.state.snapshot.sessionId===sessionId)captureInputs.select(value);
    }catch(error){
      toasts.show({key:`recording-microphone-selection-${sessionId}`,level:"error",message:`マイクを切り替えられませんでした: ${String(error)}`});
    }finally{setCaptureBusy(false);}
  };
  const startCaptureTest=async(source:HTMLElement)=>{
    if(!captureOwner||!captureInputs.selected||!captureScope.scope||root.current.baseWriteLocked||starting.current||closing.current)return;
    const owner=captureOwner,uid=captureInputs.uid,scope=checkedCaptureScope(captureScope.scope,captureScope.applications),scopeKey=JSON.stringify(scope);
    const sameInputs=()=>root.current.screen==="recording"&&(root.current.preparation?.id??root.current.selectedMeeting?.id)===owner&&root.current.captureInputs.uid===uid&&JSON.stringify(root.current.captureScope.scope)===scopeKey;
    starting.current=true;setCaptureBusy(true);setCaptureTestError("");
    try{
      const previous=await captureTest.client.refresh();
      if(previous?.busy)throw Error("前のテストの停止・検証を待ってください。");
      const importing=await invoke<ImportSnapshot>("get_audio_import");assertCloseImport(importing);
      const microphone=usableMicrophone(checkedCaptureInputs(await invoke("get_capture_inputs")),uid);
      if(!microphone)throw Error("選択したマイクまたは権限を再確認してください。");
      const apps=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,apps);
      const app=captureApplicationScope(scope,apps);
      return await startConfirm({opener:source,inlineErrors:true,title:"5秒の録音前テストを開始しますか？",target:`${microphone.name} ／ ${scope.mode==="system"?"PCの再生音全体":app?.name}`,confirmLabel:"5秒テストを開始する",
        message:"マイクとPCの音を一時保存します。参加者の同意と録音権限を確認してください。入力の初期化が終わってから5秒録音し、停止後に各音声を再生できます。会議・解析結果は作りません。解析とモデル準備は安全な停止を待ちます。"+
          (captureInputs.mode==="automatic"?"この5秒テストはOSの既定マイクを確認します。通常録音で会議アプリの別のマイクが選ばれた場合、その入力の検証にはなりません。":"")+
          (scope.mode==="system"?"PC音声には通知音や他の動画の音も含まれます。":"選択したアプリを取得します。ブラウザの他のタブの音も含まれる場合があります。終了・再起動時に別アプリへ切り替えません。")+
          (previous?"前のテスト音声は再生を停止して破棄します。":"")+(importNeedsConsent(importing)?"確認した音声取り込みを安全に取り消し、元ファイルと保存済み会議を保持します。":""),
        validate:()=>sameInputs()&&!root.current.baseWriteLocked&&root.current.state.kind==="ready"&&["idle","error"].includes(root.current.state.snapshot.phase),
        action:()=>withConfirmedEdits(withSavedEdits,async()=>{
          const [recording,currentImport,inputs,applications]=await Promise.all([
            invoke<RecordingSnapshot>("get_recording_snapshot"),invoke<ImportSnapshot>("get_audio_import"),
            invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null),
          ]);
          assertCloseRecording(recording);assertCloseImport(currentImport);checkedCaptureScope(scope,applications);
          if(!sameInputs()||recordingActive(recording)||!usableMicrophone(inputs,uid)||currentImport.token!==importing.token||!importing.busy&&(currentImport.busy||currentImport.revision!==importing.revision))throw Error("確認した入力または取り込み状態が変更されました。テストは開始していません。");
          await stopAllPlayback();
          const current=await captureTest.client.refresh();
          if((current?.runId??null)!==(previous?.runId??null)||current?.busy)throw Error("前のテストの状態が変更されています。");
          if(current)await captureTest.client.discard(current.runId);
          await captureTest.client.start({runId:crypto.randomUUID(),microphoneUid:uid,captureScope:scope,expectedImport:{token:importing.token,revision:importing.revision,busy:importing.busy}});
        }),onError:cause=>setCaptureTestError(String(cause))});
    }catch(cause){setCaptureTestError(String(cause));}
    finally{starting.current=false;setCaptureBusy(false);}
  };
  const cancelCaptureTest=async()=>{
    const id=captureTest.snapshot?.runId??captureTest.pending?.runId;if(!id)return;
    setCaptureTestError("");
    try{await captureTest.client.cancel(id);}catch(cause){setCaptureTestError(String(cause));}
  };
  const discardCaptureTest=async()=>{
    const id=captureTest.snapshot?.runId;if(!id)return;
    setCaptureTestError("");
    try{await stopAllPlayback();await captureTest.client.discard(id);}catch(cause){setCaptureTestError(String(cause));}
  };
  const stopRecording = async (source?:HTMLElement) => {
    await recordingDecision.request(false,source);
  };
  // The caller has already flushed edits. This only creates local preparation.
  const prepareNewRecording=async()=>{
    const current=await invoke<RecordingSnapshot>("get_recording_snapshot");assertCloseRecording(current);
    if(recordingActive(current))throw Error("録音状態が変更されています。最新のヘッダーから録音画面を開き直してください。");
    setPreparation(newRecordingPreparation(crypto.randomUUID(),new Date()));setSelectedMeeting(null);setCaptureError("");setScreen("recording");
  };
  const confirmPreparationResult=async()=>{
    const expected=preparation;if(!expected?.submitted||navigationLocked)return;
    await withSavedEdits(async()=>{
      const [meeting,recording]=await Promise.all([invoke<CaptureMeeting|null>("get_meeting",{id:expected.id}),invoke<RecordingSnapshot>("get_recording_snapshot")]);
      assertCloseRecording(recording);
      if(root.current.preparation?.id!==expected.id||root.current.screen!=="recording")throw Error("録音準備が変更されています。別の会議には切り替えていません。");
      if(!meeting)throw Error("保存済みの会議は確認できません。開始は再送していません。録音状態を確認し、会議一覧から新しく準備してください。");
      if(meeting.id!==expected.id||meeting.title!==expected.title.trim()||meeting.sourceType==="audio_import"||!["draft","recording","recorded"].includes(meeting.lifecycle)||recordingActive(recording)&&recording.meetingId!==expected.id)throw Error("録音の受付内容が一致しません。別の会議や録音は採用していません。");
      setSelectedMeeting(meeting);setRenameTitle(meeting.title);setPreparation(null);setMeetings(await invoke<CaptureMeeting[]>("list_meetings"));setCaptureError("");
    });
  };
  return { root, startRecording, selectRecordingMicrophone, startCaptureTest, cancelCaptureTest, discardCaptureTest, stopRecording, prepareNewRecording, confirmPreparationResult };
}

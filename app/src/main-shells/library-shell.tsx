import { lazy, useLayoutEffect, useState, type ComponentProps, type ReactNode } from "react";
import { IconButton } from "../action-icon";
import { InfoPopover } from "../info-popover";
import { isTauri } from "../ipc";
import { JobList } from "../job-list";
import { RowMenu } from "../row-menu";
import { SettingsFooter } from "../settings-footer";
import { ThemeSettingsPanel } from "../theme";
import { IssueNotice } from "../toast";
import { VocabularyEditor } from "../vocabulary";
import { DeferredPanel } from "./deferred-panel";

const VoicePeopleSettings = lazy(() => import("../voice").then(module => ({ default: module.VoicePeopleSettings })));
const MinutesPromptSettings = lazy(() => import("../minutes-prompt-settings").then(module => ({ default: module.MinutesPromptSettings })));
const InternalLogs = lazy(() => import("../internal-logs").then(module => ({ default: module.InternalLogs })));
const ModelInspectionPanel = lazy(() => import("../model-inspection").then(module => ({ default: module.ModelInspectionPanel })));

export interface LibraryPanelsProps {
  showMeetingList: boolean;
  settingsVisible: boolean;
  promptsVisible: boolean;
  settingsReady: boolean;
  locked: boolean;
  audioImport: ReactNode;
  model: Omit<ComponentProps<typeof ModelInspectionPanel>, "visible" | "locked">;
  dataSave: ReactNode;
}

// 共通の削除ダイアログより前の位置に固定し、モデル設定とプロンプトは初回表示後もマウントを維持する。
export function LibraryPanels({
  showMeetingList, settingsVisible, promptsVisible, settingsReady, locked, audioImport, model, dataSave,
}: LibraryPanelsProps) {
  const [openedSettings, setOpenedSettings] = useState(false);
  const [openedPrompts, setOpenedPrompts] = useState(false);
  useLayoutEffect(() => {
    if (settingsVisible) setOpenedSettings(true);
    if (promptsVisible) setOpenedPrompts(true);
  }, [settingsVisible, promptsVisible]);

  return <>
    {showMeetingList && <div className="page-heading"><h1>会議一覧</h1></div>}
    {audioImport}
    {settingsVisible && <div className="page-heading heading-with-info"><h1>設定</h1><InfoPopover label="設定">録音環境、ローカルAI、表示を管理します。</InfoPopover></div>}
    {settingsVisible && <ThemeSettingsPanel locked={locked} />}
    {(openedSettings || settingsVisible) && <DeferredPanel visible={settingsVisible}>
      <ModelInspectionPanel {...model} visible={settingsVisible} locked={locked} />
    </DeferredPanel>}
    {promptsVisible && <div className="page-heading heading-with-info"><h1>プロンプト</h1><InfoPopover label="プロンプト">議事録の作成・統合に使うプロンプトと共通用語を管理します。</InfoPopover></div>}
    {(openedPrompts || promptsVisible) && <DeferredPanel visible={promptsVisible}>
      <MinutesPromptSettings visible={promptsVisible} locked={locked} />
      {promptsVisible && <VocabularyEditor locked={locked} />}
    </DeferredPanel>}
    {dataSave}
    {settingsReady && <SettingsFooter />}
  </>;
}

interface MeetingRow {
  id: string;
  title: string;
  createdAt: number;
  status: string;
  restored: boolean;
  recoveryWarning: boolean;
  retry: ReactNode;
  menu: ReactNode;
  deletionPending: boolean;
}

interface LibraryMeetingListProps {
  hidden: boolean;
  locked: boolean;
  meetingError: string;
  restoredOnly: boolean;
  search: { query: string; update: (value: string) => void };
  list: {
    rows: readonly MeetingRow[];
    hasMeetings: boolean;
    status: "loading" | "ready" | "failed";
    error: string;
    diagnosticError: string;
    diagnostics: readonly { reason: string; message: string }[];
  };
  actions: {
    select: (id: string) => void;
    reload: () => void;
    clearRestoredFilter: () => void;
    trackMenu: (id: string, open: boolean) => void;
  };
}

export type LibraryShellProps =
  | ({ screen: "meetings" } & LibraryMeetingListProps)
  | { screen: "voices"; locked: boolean }
  | { screen: "logs" | "settings" | "prompts" }
  | { screen: "jobs"; jobs: ComponentProps<typeof JobList> };

export function LibraryShell(props: LibraryShellProps) {
  switch (props.screen) {
    case "voices":
      return <DeferredPanel><VoicePeopleSettings standalone locked={props.locked} /></DeferredPanel>;
    case "logs":
      return <DeferredPanel><InternalLogs /></DeferredPanel>;
    case "jobs":
      return <JobList {...props.jobs} />;
    case "settings":
    case "prompts":
      return null;
    case "meetings":
      return <LibraryMeetingList {...props} />;
  }
}

function LibraryMeetingList({ hidden, locked, meetingError, restoredOnly, search, list, actions }: LibraryMeetingListProps) {
  return <section className="meetings-card" aria-label="保存済み会議" hidden={hidden}>
    <div className="card-head"><input type="search" className="meeting-search" aria-label="会議名を検索" placeholder="会議名を検索" value={search.query} onChange={event => search.update(event.target.value)} /><IconButton label="再読込" icon="refresh" onClick={actions.reload} disabled={!isTauri() || locked} /></div>
    {!hidden && <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} />}
    {restoredOnly && <p className="meeting-list-notice">今回復元した会議を表示しています。<button type="button" className="small-secondary" data-screen-navigation disabled={locked} onClick={actions.clearRestoredFilter}>すべての会議を表示</button></p>}
    {list.status === "loading" && <p className="empty-meetings" role="status">保存済み会議を読み込んでいます。</p>}
    {list.status === "failed" && <p className="meeting-list-notice" role="status">会議一覧を取得できませんでした。{list.hasMeetings && "前回取得した一覧を表示しています。"}再読込してください。</p>}
    <IssueNotice scope="main" type="meeting-list" label="会議一覧" message={list.error} />
    {list.status === "ready" && list.diagnosticError && <p className="meeting-list-notice" role="status">会議の利用状態を取得できませんでした。会議一覧の「再読込」を選んでください。</p>}
    {list.diagnostics.map(item => <p key={`${item.reason}:${item.message}`} className="meeting-list-notice" role="status">{item.message}</p>)}
    {list.rows.length === 0 ? list.status === "ready" && <p className="empty-meetings">{search.query ? "条件に合う会議はありません。" : restoredOnly ? "対象の復元会議は現在の一覧にありません。" : "保存済み会議はありません。"}</p> : <div className="meeting-table-frame">
      <table className="meeting-table"><thead><tr><th scope="col">会議名</th><th scope="col">作成日時</th><th scope="col">状態</th><th scope="col"><span className="sr-only">操作</span></th></tr></thead>
        <tbody>{list.rows.map(row => <tr key={row.id}>
            <td><button type="button" className="meeting-name" data-screen-navigation onClick={() => actions.select(row.id)} disabled={locked}>{row.title}</button>{row.restored && <small className="restore-badge">バックアップから復元</small>}</td>
            <td className="meeting-date">{new Date(row.createdAt).toLocaleString("ja-JP")}</td>
            <td><span className="state-pill">{row.status}</span>{row.recoveryWarning && <small role="status">保存状態を確認できません。表示は前回取得した状態です。</small>}</td>
            <td><RowMenu label={`${row.title}の操作`} disabled={locked} onOpenChange={open => actions.trackMenu(row.id, open)}>
              <button type="button" role="menuitem" data-screen-navigation onClick={() => actions.select(row.id)} disabled={locked}>開く</button>
              {row.retry}{row.menu}{row.deletionPending && <span role="status">会議の削除が完了していません。</span>}
            </RowMenu></td>
          </tr>)}</tbody>
      </table>
    </div>}
  </section>;
}

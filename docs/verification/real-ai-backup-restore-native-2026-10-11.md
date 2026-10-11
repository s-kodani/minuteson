# 実AI結果を含むMac QAバックアップ／追加復元

2026-10-11。M3-06／M4-01の正常系証拠を追加。正式評価・Windows固有部分は保留。既存の実AI30秒会議を使用し、新規録音・AI再実行・外部送信・本番データ変更・削除は行わない。

## 実行した通常UI経路

使用QAは `/private/tmp/mon-spacing-select-qa.lFJAwI/MON QA.app`（main SHA `0e3f47a30688de1ddf2ae27d9bf25f074ac04be7192486cafded861b99b00a85`）。既存の移動済みQA保存先で4会議・解析版4・下書き2を確認して開始した。DBへ試験結果を直接投入せず、作成／復元はCUAの通常UIだけで実行。

1. 設定→バックアップを作成→OS保存画面で新規専用フォルダーとファイル名を選択。完了表示は4会議・2.8MiB。保存先は `/private/tmp/mon-real-ai-backup-qa.qTMnSr/real-ai-roundtrip.monbackup`、実サイズ2,942,807bytes、SHA-256 `4a31163006bcaf1bc143e9480e520098a546757efa2187745b6b85e07682e6a1`。
2. 同ファイルをOSのファイル選択から追加復元へ渡す。4会議の追加・既存を置換しないこと・必要容量24.5MiB・モデル本体を追加しないことを確認。
3. native AXで確認画面の見出し・説明・ボタンを取得。キャンセル初期フォーカスからTabで会議一覧のsummary、次のTabで「追加して復元」へ到達し、Returnで1回実行。以前の旧QAで観測した追加復元確認のAX欠落は、今回の経路では再現しなかった。ただし実VoiceOverや全モーダルの保証とはしない。
4. 復元した会議一覧の4件と由来表示を確認。復元した実AI会議の文字起こし2発言・保存済み、議事録の編集マーカー・現在版#1・保存済み編集を含む表示を確認した。
5. 議事録の根拠から固定元発言・前後の文脈・認識原文を表示。参照記録で復元先の固定解析版 `cec17182-0508-4bc0-82c0-948039d2c0d4` と発言 `bf114a56-a98a-4981-88b5-f12a3887fdfb` を確認。19秒区間の再生開始、0:19／0:19の終点と停止表示を確認して再生を閉じた。品質・同期精度の評価や確認状態変更は行わない。
6. Cmd-Q後のmain不在を読み取りプロセス照合で確認し、同QAを冷起動。元4件＋復元4件の8会議と由来表示を確認。2行目の復元由来の実AI会議名へキーボードで到達したことを画像確認して開き、文字起こし・議事録の保存済み状態と編集マーカーを再確認。最後もCmd-Qとmain不在を確認した。

画像は同専用フォルダーの `cold-start-eight-meetings.png` と `original-and-restored-after-restart.png`。会議一覧の画像だけを残し、実録音の本文を最終返信へ添付しない。

## 読み取り専用の内容照合

復元バッチ `7c04fd2f-22c6-47ba-b338-351033773697` はpublished。その固定ID対応表を使い、読み取り専用SQLiteトランザクションで以下を検査した。一般の本文文字列を置換せず、構造上のIDフィールドだけ対応付ける。speaker／utteranceの行IDとlineage IDは別に扱う。

| 対象 | 照合結果 |
|---|---|
| 会議 | 4件とも新しい別ID。元の会議を保持 |
| 文字起こし・議事録下書き | 2件のJSON全体がID対応後一致。種別・schema・版・activeと基準版参照も一致 |
| 議事録版 | 1件の生成文書と有効文書のJSON全体がID対応後一致。入力解析版・版番号・公開状態等も一致 |
| 編集履歴 | 2件のpayload／inverseのJSON全体、操作種別・順序・作成時刻と復元先下書き参照が一致 |
| 固定根拠 | 12件の全列がID対応後一致。引用・時刻・確認状態を保持し、元会議の版へ戻る参照ではない |
| 管理音声 | readyの8実ファイルで元／復元の形式・サイズ・記録SHAが一致し、復元先ファイル自体のSHAも一致 |
| 一時PCM | 2件の本体が存在せず、missing状態とrecreatable_pcm除外理由・元サイズ／SHAの記録が一致 |

再実行用の読取検査 `/private/tmp/mon-real-ai-backup-qa.qTMnSr/verify_restored_content.py`（SHA `cbc4646240ce1d38d4e23e1667d7e35f4aa41dab3401dab609da663743114874`）を保持。DBへの書込み・モデル実行・新規データ生成を行わず、本文は出力しない。実行中、最初の通常終了後、冷起動後の最終終了後に成功した。

既存のworking_drafts／minutes_revisions／evidence_links／meetingsを起動前にid順のSQLite JSON出力でSHA照合し、復元後と冷起動後も `15c7d35f56e86ad968779ef8f109ffd6f36ffa1922f8898884db9e2f6d86e8ba` で不変。これはこの4表の元対象の完全出力一致であり、全DB・全履歴表の不変証明ではない。最終integrity_check=ok、foreign_key_check=0行。最終会議8／解析版8／議事録版2／下書き4／根拠24／編集履歴4。

モデルはhistorical_snapshot／unavailableの10パッケージとunavailableの10依存ファイル記録が追加された。管理モデルの本体保存先は未作成、適用プロファイル0。復元した履歴情報を実モデル配置・互換性確認・AI準備完了には数えない。ジョブはsucceeded6件で新規モデル実行を始めていない。bootstrapは前後同じSHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020`。バックアップ自体のSHAも通常終了後に一致した。

## 検査時の失敗と未達条件

- 初回の検査スクリプトが会議IDのsource_scopeを会議固有と仮定したためKeyError。全体scopeの対応を確認して修正。製品復元は変更せず、この初回を成功扱いにしない。
- 次の検査は除外されるPCMまで実ファイル存在を要求して失敗。missingと除外証拠のスキーマを確認し、理由・サイズ・SHA・本体不存在を検査する経路へ修正した。別のSELECTで除外表に存在しないbatch_id列を指定した試行も検査成功には使わない。
- OS保存画面の初期名に `.monbackup.monbackup` が表示される観測があった。今回は利用者入力相当で単一拡張子へ直して保存した。製品の既定名とファイル種類設定順序の調査／修正は残件で、名前を直さない既定保存経路の合格にはしない。
- 全旧形式・破損／改変・中断／電源断・未完了回収・管理モデルが実配置された状態・実VoiceOverの全条件を今回の正常系に置き換えない。M3-06／M4-01と計画全体は進行中。録音／TCC／4時間、署名／公証／手動更新・先行試用などは残る。

今回の製品ソース変更は0で再ビルド0回（0秒）。通常Rust／フロント全件／Swift全件は再実行せず、前ターンの694件を今回の検査へ加算しない。GUI・調査・内容照合・記録の個別時間は未計測。専用検証フォルダーは3,456KiB、共有Rustキャッシュ8,420,760KiBのまま。既存QA・元／復元会議・元音声・外部モデルは保持し、CI・コミット／Pushなし。

## 保存ダイアログの既定名修正と追加検証

2026-10-11の追加。上記の拡張子重複は歴史的な失敗観測として保持する。未コミットの `exports.rs` では、NSSavePanelの許可拡張子を設定した後、表示直前に完全な既定名を設定する順序へ変更済み。受け取ったURLのパスや拡張子を後から書き換えず、Cancel→None、ローカルURL必須、原子的公開・既存対象の変更拒否は維持する。バックアップとMarkdownは同じhelperを使う。

先行QA `/private/tmp/mon-save-dialog-qa.Mzb9Nd/MON QA.app`（構築194.44秒）で、既定名を編集せず保存先フォルダーだけを選び、バックアップ8会議の保存と続けてMarkdownの保存を行った。結果の実ファイルは以下。以前の手動拡張子修正による回避とは区別する。

| ファイル | bytes | SHA-256 |
|---|---:|---|
| `/private/tmp/mon-save-dialog-output.mfBtU3/MON-20261011-004606.monbackup` | 5,936,314 | `6f3882859d86ef3cb73975c75c51204e9a0c56a9615b38e71ed100d9078cc382` |
| 同フォルダーの `2026-10-10_QA Real AI 30s 2026-10-10_議事録.md` | 5,159 | `fa23384fefc6070c491a2d21b737f8fa7e681414f49e4e1a4b5643ecf97bf034` |

今回、最新UIを含む `/private/tmp/mon-spacing-import-qa.B0HkEA/MON QA.app`（main `611fdf0e9e3ccf7b9fba1c767332fb46f9af89dc2fe2450731e4347c6da3870c`、strict deepアドホック署名）を冷起動。既存の実AI会議でMarkdown保存画面を開き、単一 `.md` を確認して取消。設定のバックアップ保存画面へ続けて切り替え、既定名 `MON-20261011-012053.monbackup` に単一拡張子があることと `.md` の混入がないことを確認して取消。既定名の入力編集なし。画像 `/private/tmp/mon-save-dialog-markdown-cancel-final.png` と `/private/tmp/mon-save-dialog-backup-after-markdown-final.png`。先行のバックアップ→Markdownと今回のMarkdown→バックアップで双方の切り替えを確認。今回の逆方向は取消の証拠であり、新しい保存／復元成功には数えない。

readonly DBの出力記録 `dbec2d99-b2f1-46f1-9cea-e012570f18e4` はsucceeded／markdown。保存ファイルのサイズ・SHAは記録と一致。出力元は議事録 `f1412360-9a02-4cee-9021-04d627cf8388` と下書き `044bf2ab-371c-41ad-a1f2-85f4ff8e0d08` lock1、現在の同じ会議・基底版・active下書きlock1と一致する。下書きJSON本文（末尾のCLI出力改行を除く）のSHA `c6cdf9cee6442008aaabce55254516ab386e0ba9f3807ff87aa84be6a3718c30` はexported_content_hashと一致。保存下書きのcontent_hashは `mon-minutes-draft-v1` prefix付きで別値になる設計であり、同じ値であるとの誤判定を避けた。prefix付き再計算 `4c020e0c99de62e5c0502c8f55f67101f934f6a616b2ef43cdf107d150b650ff` もDBと一致。本文は検査出力・返信へ載せない。

初回の由来照合SELECTで存在しないd.minutes_revision_idを指定して失敗。実schemaのbase_minutes_idへ修正し、基底版と会議・lockを照合した。別の比較でprefix付き下書きhashをprefixなしの出力hashと比較した結果0を観測し、実装の定義と上の2種類の再計算で区別した。これらの中間観測を合格へ置き換えない。

取消前後の全export_recordsをid順JSON出力したSHAは `caab02d859121526ce97226a517c500d71cac9a4bbd7e815bf48e02695d3c19d` で不変。OS表示先 `/private/tmp/mon-native-ui-smoke.SOSoB9` の取消対象Markdown・バックアップと重複拡張子のファイルは不在。既存2出力のSHAは不変。終了後DB整合性ok／外部キー0行、会議8／解析版8／議事録版2／下書き4／出力記録3。bootstrap SHAは上記のまま、Cmd-Q後に指定QAのmain不在を確認。

保存画面のsource contract2件を重点再実行して成功（runner0.068秒）。先行の関連Rust10件は保存・取消・変更対象拒否・原子的公開・出力由来・応答喪失等を検査し成功、保存ダイアログ自体のOS挙動は今回のGUI証拠へ分ける。ログ `/private/tmp/mon-save-dialog-rust-tests.log` はbuild387.98秒／execution8.25秒／テスト本文0.50秒、終了後cache8244.8MiBで、今回の新規実行ではない。最新フロント700件・QA186.63秒も前のUI点検の実績であり今回へ二重加算しない。今回の製品変更・再構築・通常Rust全件・CI・コミット／Pushは0。正常保存と取消の既定名問題を解消したが、全旧形式・破損・中断・実IME／VoiceOver・実配布条件と計画全体の未達は維持する。

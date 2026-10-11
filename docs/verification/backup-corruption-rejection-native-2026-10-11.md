# Mac QAの破損バックアップ拒否と選択入口への復帰

2026-10-11。M3-06／M3-08／M4-01の非録音異常系を追加確認。正式評価・Windows固有部分は保留。実会議の編集・録音・AI実行・モデル取得・OS設定変更・本番保存先の操作・CI・コミット／Pushは行わない。

## 検証素材と変更範囲

既存QAの正常バックアップ `/private/tmp/mon-real-ai-backup-qa.qTMnSr/real-ai-roundtrip.monbackup`（2,942,807bytes、SHA `4a31163006bcaf1bc143e9480e520098a546757efa2187745b6b85e07682e6a1`）を読取り専用で使用。再現用補助ツール `tools/mon_backup_corruption_fixtures.py` は最大64MiB・通常ファイル・leaf symlink拒否・安定したfstat、形式／manifest／全payloadのSHAを確認してから、新規private directoryへ4つの不正コピーを作る。元ファイルを更新せず、既存出力を上書きしない。これはアプリの意味・参照・DB整合検査の代替ではない。

```
python3 tools/mon_backup_corruption_fixtures.py /private/tmp/mon-real-ai-backup-qa.qTMnSr/real-ai-roundtrip.monbackup --parent /private/tmp
```

素材は `/private/tmp/mon-backup-corruption-qa.zx0qwf3h`（11,504KiB）に保持。元バックアップと4コピーのSHAは実機操作後も一致。壊れた素材を取り込んだり、検査を通すために補修したりしない。

| コピー | 変更箇所 | 最終Mac UIでの拒否表示 |
|---|---|---|
| wrong-magic.monbackup | 先頭8bytesだけ別識別子 | MONバックアップ形式ではありません |
| truncated.monbackup | 最後の1byteだけ欠落 | バックアップの全体容量・末尾が不正です |
| manifest-digest.monbackup | ヘッダーの期待manifest SHAの1bit | バックアップmanifestハッシュが一致しません |
| payload-digest.monbackup | 最後のpayloadの1bit | バックアップの管理ファイルハッシュが一致しません |

## 発見した操作不具合と修正

先行QA `/private/tmp/mon-backup-rejection-qa.ykPttP/MON QA.app`（main SHA `96080315eb07f96faca38d9a0772b4fdca01c3b64ac1524a941506eda7129f97`）で不正形式／末尾欠落は正しく拒否されたが、native選択画面を閉じた後のAX焦点が会議ワークスペースcontainerへ落ちた。失敗直後にはavailabilityの1秒pollより前のdisabled表示も観測し、状態再確認後にenabledとなることを確認。これを恒久的なロックと断定しない。

`DataSavePanel` の作成／追加復元のclickから実buttonを渡し、native選択前に明示focusして入口を保持。RPC応答はnative画面完了ではなくbusy選択受付なので、応答のtokenを保持し、同じtokenのidle（OS選択取消）／failed／cancelled／completedと、canAct再有効化のlayout commitを待って復帰する。確認モーダル中・未知結果・別token・非表示・disabled・hidden／inert・別modal・別の有効操作focusは奪わない。focusのために取得／復元／作成命令を再送しない。保存待ち・明示確認・容量／SHA／原子的公開・既存会議保護は変更しない。

最終QA `/private/tmp/mon-backup-rejection-focus-qa.xB15pi/MON QA.app`（main SHA `8e634f414767c97fd81c80983ba335127f7be7f8f055756bf0bad9cc03e06096`、strict deepアドホック署名成功）で、通常設定→追加復元→OS選択→Openの4種類を順に実施。全て拒否し、確認／追加実行操作を提示せず、有効な「バックアップから追加復元」へAX焦点が戻った。手動の「状態を再確認」を押して復帰させていない。

さらに追加復元のOS選択Cancel、バックアップ作成のOS保存Cancelでそれぞれ入口focusを確認。正常な元バックアップでは「会議4件を追加・現在を置換しない」確認モーダルと安全なcancel初期focusを確認し、追加実行せず「取り消して閉じる」で入口へ戻った。終了→対象main不在→冷起動で11会議と正常な設定の作成／追加復元入口を確認し、再度終了した。最終画像 `/private/tmp/mon-backup-rejection-focus-native-20261011.png` はpayload拒否表示。検査中のGo to Folder補完がまだ確定していない入力／Escape等の中間操作は拒否成功へ数えない。現在の選択一覧を読み直して実ファイル名を選択してからOpenした結果だけを採用する。

## データ保護の読取り照合

起動前に残っていた旧QA2プロセスは同じ起動設定の排他診断画面であり、通常Cmd-Qで終了。DB・WAL・SHM・bootstrapを削除しない。対象QAとAI実行プロセス不在を確認した後にだけ外部SQLiteのreadonly照合を行った。

起動前・最終通常終了後・冷起動確認の最終終了後で以下のid順JSON連結SHAが一致。最初の追加検査SQLではrestore_batchesに存在しないid列を指定して失敗し、その空出力を証拠にしない。実schemaのbatch_id等で訂正してからGUI検査を開始した。

| 保護対象 | 開始／終了共通SHA-256 |
|---|---|
| 全working_drafts／export_records | `54e5ff1ce509a1d11ce1db11fd58ae5788e6f40337c050c795a19862215e94f7` |
| 全解析／議事録版と11会議の現在／直前参照・title | `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961` |
| 全app_settings／processing_jobs | `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226` |
| 全用語entries／scopes／snapshots | `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee` |
| 全restore_batches／identity_maps／provenance／edit_operations | `3606dd2488222bac31f20b10a4756644ad206d954f77f722b782f1658569bd9d` |

追加表の順序はbatch_id、batch_id/domain/source_scope/source_key、id、id。整合性ok・外部キー違反0、会議11／復元batch2を保持。bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` と元バックアップSHAも一致。この検査対象以外の全DB bitwise不変を主張しない。

## 検査結果・工程と残件

- 最終フロント787成功・失敗／skip0、本体1.704秒。ログ `/private/tmp/mon-backup-rejection-focus-final-tests-20261011.log`、SHA `ab44fea7480cba60a8a91fcfc40e0293adef1dd1fa9fd356011ffa0c24402d2c`。
- Python全体132成功、11.327秒。新補助ツールの5件を含み、別の5件先行実行を重複加算しない。ログ `/private/tmp/mon-backup-corruption-fixture-python-20261011.log`、SHA `2a747c2329d247b54251777fe6cdd6abca7f52229392a04a43b9e98219624855`。compileall／driver構文／diff whitespace成功。
- 最終QA型検査成功・Vite1.44秒・Cargo1分09秒。ログ `/private/tmp/mon-backup-rejection-focus-final-qa-build-20261011.log`、SHA `a0aaca39604689e2b6e4a890ee04562006d06f2678e6990a09edda3020d71d67`。先行調査QAはVite1.49秒・Cargo1分10秒。途中buildはtopOpenModalへ不要な引数を渡した型エラーで終了1、修正後の最終buildで成功。並行実行・型検査／bundling等の未分離を含むため、表示時間を全作業経過へ単純合算しない。調査／実装／GUI／記録は個別未計測。
- フロントsource契約にsame-token／terminal／enabled／modalと非再送guardを追加。既存browser driverにもinvalid拒否／OS取消の入口focus期待を追加して構文確認したが、CLI全行程の合格とはしない。今回Rust製品ソース変更0、通常Rust22分全体／Swift全体は再実行しない。前回の最新通常Rust1,829成功は今回の新成功数へ加えない。

M3-06の4種類の破損拒否と取消／冷起動の証拠であり、全旧形式・容量不足・中断／電源断・未完了回収・全攻撃的破損を網羅した証拠ではない。実IME／VoiceOver、実録音／4時間、配布のDeveloper ID／公証／notice承認、モデルの実TLS／CDN通常取得、少人数の実会議試用等の残件は維持。前回の追加CSSを含む最新UIは今回最終QAへ反映済みだが、全画面の実機完了宣言にはしない。目標は進行中。

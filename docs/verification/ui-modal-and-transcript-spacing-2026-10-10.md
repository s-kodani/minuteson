# 設定モーダルと文字起こしの余白・操作配置

## VoiceOver検証の対象外指定

2026-10-11のユーザー指定により、VoiceOver検証は以後行わず、完了条件・残タスク・残工数から除外する。以下のVoiceOver未検証の記載は指定前の検証履歴で、検証合格を意味しない。日本語入力と通常のキーボード操作は引き続き対象とし、アイコンの操作名・ラベルは保持する。[計画の範囲変更](../development-plan.md#2026年10月11日の検証範囲変更)。

## 2026-10-11 アイコン指定の最終点検と速度欄の重なり修正

ユーザー指定の×／編集／pause／先頭／三点／更新と、追加・undo・前後移動は既存の共通SVG実装を維持。CUAの独立合成タブで、空のボタン本文・SVG・titleのホバーラベル・読み上げ名、通常40px／toolbar32pxを確認。編集オプションを開閉し入口focus復帰、発言再生→一時停止でラベルが再生へ戻る動作を確認した。保存・削除・採用等の判断を伴う操作は文字を残す。

再生バーで速度labelが36.87pxへ縮む一方、内側selectが74pxあり、隣の時刻表示へはみ出す問題を実測。labelを縮小しない一列構成、selectを隣接ボタンと同じ32px（通常行40px）のborder-box高さに変更。controlsのflex-basisを320pxにして小画面で操作群全体が極端に狭い列へ縮むことも防止した。最終1280pxではselect74×32px・label82×32px、時刻との間隔8px、各アイコン32×32pxで重なりなし。360pxでは操作を2行に折り返し、横溢れ0・selectと時刻の重なりなし。viewportは元に戻し、確認用タブは終了時に閉じる。画像 `/private/tmp/mon-icon-request-ui-20261011.png`。

最終フロント786件成功・失敗／skip0、test1.740秒（ログ `/private/tmp/mon-icon-request-verification-20261011.log`）。型検査と本番build成功、Vite1.51秒（ログ `/private/tmp/mon-icon-request-build-20261011.log`）。バンドル500kB超の既存警告は残る。今回Rust／native QA再構築は行わず、保持中QAにはこの追加CSSは未反映。実会議・DB編集・録音・OS設定変更・CI・コミット／Pushなし。全UI／IME／VoiceOverや配布等の残条件を完了扱いにしない。

## 2026-10-11 キーボードの本文開始と出力取消focus

仕様14.14／UX-11に沿ってキーボード経路を点検。本文のrole=buttonで独自に処理していたEnter／SpaceはisComposingのみを見ており、keyCode229・repeat・defaultPreventedを除外していなかった。`shouldOpenTranscriptBodyFromKey`で両IME信号・長押し・既処理・全modifierを除外し、plain Enter／Spaceだけ編集開始を許可する。ポインターの本文選択保持と保存・同意・下書き条件は変更しない。純粋関数3件と実際の接続contractを検査し、初回の旧source期待1件を新helper接続＋editable条件の期待へ更新した。これを実IME入力試験と混同しない。

CUA合成でCmd+F検索focus（2px枠）、Enter／Spaceによる本文編集、合成訂正のCmd+Enter確定→行focus（2px枠）→Cmd+Z元本文復帰、別入力のEsc取消→元本文・行focusを確認。RAFによる編集欄focusは次の描画後に確認し、即時観測だけを初期focusの証拠としない。設定のShift+Tab最終有効操作→Tab安全な×（3px枠）→Esc入口復帰、End→Spaceの議事録タブ切替も確認。下書きなしの割当selectはdisabledなのでTab対象にせず、有効な回復入口から循環を確認した。画像 `/private/tmp/mon-keyboard-modal-browser-20261011.png`。visible controlsのCLI driverを追加・登録・構文検査したが、CLI suiteは未実行。

先行QA `/private/tmp/mon-keyboard-ime-qa.FXWcBR/MON QA.app`（main SHA `fc3331dd4ecaf1b0105e50a740e9cf5b17d45520de68e7a143ae5897b0186e1f`）のMacで、Cmd+F会議検索→Tab再読込→Tab自作会議→Enter詳細、Cmd+F発言検索→Tab本文領域／行／時刻／本文→Enter編集欄focus→Esc行復帰を確認。本文は変更しない。Spaceで自作発言の保存音声再生を確認したが、3秒区間が観測間に終了し得るため、次のSpaceを確実なpause検証へ数えない。Cmd+Nで録音準備名へfocus、Shift+Tab→Spaceで一覧へ戻ることを確認し録音は開始しない。設定のShift+Tab／Tab循環、infoをSpace→Escで親入口→Esc元入口、左右／Enterで議事録切替、出力へTab／SpaceでSave panel、Esc取消、議事録からCmd+F発言検索も確認。画像 `/private/tmp/mon-keyboard-modal-native-20261011.png`。先行出力取消後のfocusがHTML contentとなる問題を追加発見した。

出力開始時に実際のbuttonを保持し、既知のexport応答またはactive=falseの結果確認後だけ復帰を予約。busyのdisabled renderが解除されたlayout commitで、focusがbody／元button／切断対象にある場合に入口へ戻す。別の有効入力・非表示view・独立modal・進行中または結果不明のnative操作を奪わず、focusのために出力命令を再送しない。

最終QA `/private/tmp/mon-keyboard-export-focus-qa.ZkcwFl/MON QA.app`、main SHA `d875651cca2c4018a9c330c141b5c7656b83f8bded85673758418910937d0eff`、ad-hoc deep／strict終了0。Macの自作会議でTab→出力Space→native Save panel準備完了を確認→Esc取消を2回実施し、毎回Markdown出力buttonへfocus復帰。初回のpanel初期化前Escapeはpanelが残ったため取消証拠とせず、準備完了後に再検査した。ファイル保存・新しい出力履歴なし。画像 `/private/tmp/mon-keyboard-export-return-native-20261011.png`。実IME／VoiceOverの一時切替許可を質問済みだが回答前のためOS設定は変更していない。全主要操作・実IME／VoiceOver・両テーマ等の完了にはしない。

最終785成功・失敗0・skip0、runner2.850372917秒。`/private/tmp/mon-keyboard-export-focus-final-tests-20261011.log` SHA `2f3c902591693ba066451e978582e93b69455e8e2bb3dc484b11f4a088d0014a`。型検査／QA終了0、Vite1.47秒／Cargo1分11秒、`/private/tmp/mon-keyboard-export-focus-qa-build-20261011.log` SHA `502ee2a5feda5208264494c51e85efb8a962be8259077913b0197c10898f7306`。先行QAもCargo1分11秒。実装／手動検証時間は未分離。shared target再利用・通常Rust全体再実行なし。

前回undoのlock19を起動前基準とし、全11会議のworking_drafts＋export_records sorted JSON SHA `54e5ff1ce509a1d11ce1db11fd58ae5788e6f40337c050c795a19862215e94f7` が最終終了後に一致。全版＋会議参照・設定＋jobs・用語3表・bootstrapは前節SHA不変、対象draftはlock19／元payload SHAを保持。integrity_check ok／FK0行。回復journalの全行bitwise不変までは今回の集計対象でない。録音・実本文変更・新規出力・追加取得・正式署名／公証・CI・コミット／Pushなし、正式評価／Windows保留とgoal未完了を維持。

## 2026-10-11 補助画面のアイコン補完とundo後の設定

残っていた50個の通常の再読込・一時停止・前後移動／戻るをIconButtonへ変更。回復一覧／入力、固定文字起こし、根拠、推定区間、欠損音声、モデル記録、削除対象一覧、通知等を含む。TypeScript ASTでplain textのbuttonのみを特定し、元の属性・handler・disabledをそのまま保持した。titleとaria-labelに従来名を残し、操作は標準40px／compact32pxの共通寸法。取り消して閉じる・削除せず閉じる・保存／再試行など、結果を伴う曖昧な操作は文字を維持。全TSXをAST走査し、普通の再読込／pause／前後移動の文字buttonが残らないことを検査する回帰を追加した。

合成の回復一覧で×／refresh／前／次が本文空・SVG1個・title／aria名付き40×40px。360×800ではmodal幅328px・左16px・横溢れ0を確認し、Escapeで回復入口→編集入口へ復帰。画像 `/private/tmp/mon-icons-complete-browser-20261011.png`。CLI browser suiteや全50操作のクリックは未実行。

最終QA `/private/tmp/mon-icon-completion-qa.mCnmBd/MON QA.app`、main SHA `6f587e720092a0cbdbca26defaa96716bc212e7367a5650dd0c056362ae295b0`、ad-hoc deep／strict終了0。Mac回復一覧の前後アイコンと初期×focusを確認。自作会議だけのundo→編集設定をEscape→00:00:03発言設定を2回検査し、AX全内容・初期×focus・Escape元入口復帰が毎回成功。以前失敗したundo後の個別設定順序はこの限定経路で改善を確認した。冷起動後にも同発言の元話者割り当てと設定AXを確認。

undoは連続過去履歴へ遡るstackではなく、直前operationの往復。自作会議のdraft `dc977c24-3283-42ab-be64-f5adbd9e0508` はlock17／SHA `8120a0cc0dab61cc767d8dd4a63a7d7ad345e74507545a3ffe185a6d7fbf712c` → lock18／`a0432033497007edfb65425b0b2b2e8bece83fc9eb9efb30ad38a6798170460d` → lock19／元SHAへ復帰。元payload・話者割り当てを復元し、通常GUIのundo履歴2件を残した。検証のためDBを書き換えたり履歴を消したりしない。画像 `/private/tmp/mon-undo-settings-native-20261011.png`。冷起動後、議事録の固定全文へ移動し、前後／戻るのAX・読取り専用8発言と固定引用、戻る操作後の全文入口focusを確認。画像 `/private/tmp/mon-icons-fixed-native-20261011.png`。固定全文は1ページのため前後はdisabled、実ページ移動や全VoiceOver／IMEの証拠には拡大しない。

最終780成功・失敗0・skip0、runner2.93412175秒。`/private/tmp/mon-icon-completion-final-tests-20261011.log` SHA `d4c7e3b8b3fbf0170156281325fa546d4d8a6b043ae3aad9cb84e395d5cf971d`。型検査／QA構築終了0、Vite1.45秒／Cargo1分10秒、`/private/tmp/mon-icon-completion-qa-build-20261011.log` SHA `2b356de660426d6132ff855c83d9de620b3c8c8cdf5ad868650aeb045e450b55`。実装／手動検証時間は未分離。shared target再利用、通常Rust全体は再実行せず、新たな大容量cacheを作らない。

自作draftを除く他10会議のworking_drafts＋全export_records＋他10会議edit_operationsのsorted JSON SHA `1af3087f63bd1dcbce6530fedf166dda464b1ebc775a4e8ad15e7b1e2f1a160b` は起動前／最終冷起動終了後で一致。全11会議の版＋会議参照、設定＋jobs、用語3表、bootstrapは前節SHA不変。integrity_check ok／FK0行。検証用draftだけの版・監査履歴更新を全DB不変と主張しない。録音・AI開始・追加取得・正式署名／公証・CI・コミット／Pushなし。正式評価／Windows保留・全UI／配布等の残件とgoal未完了を維持。

## 2026-10-11 回復一覧の読込完了後フォーカス

前回残ったMac初期focusを修正。回復一覧は開くと同時にbusy=trueとなり、×がdisabledの間に初期focusを試みる。固定portalの表示より前に見出しへのfocusも失敗し得るため、replacementのkey変更だけでなく同一viewのblocked true→falseも追跡し、focusが外れている場合に安全な×へ戻す。有効な入力／操作のfocusは保持し、rAFもdisabled要素を有効focusとみなさない。保存／結果未確認／confirm所有者等のguardは変更しない。

合成画面で読込中の×disabled、focused完了ボタンが無効になった後の×focus、別の有効な完了ボタンのfocus保持を確認。Escapeは読込入口→workspaceへ復帰し、実行回数0。driverへ両ケースを追記しsource検査を追加したがCLI全体は未実行。画像 `/private/tmp/mon-modal-ready-browser-20261011.png`。

QA `/private/tmp/mon-modal-ready-qa.HkC7qo/MON QA.app`、main SHA `31eff63e3495ff6659f4c36e2d846a22f3412baa7a46e8911e812f46043384ad`、ad-hoc deep／strict終了0。Mac冷起動→自作合成会議→再生設定→×→編集設定→回復一覧で×初期focusを確認。Tabで再読込へ移動し、Escapeで回復入口・さらに編集入口へ復帰。追加2反復でも×初期focusと回復入口復帰を確認（計3回）。画像 `/private/tmp/mon-modal-ready-native-20261011.png`。前節のMac初期focus残件はこの検証前の履歴。undo後の個別設定・全VoiceOver／IME／異常順序の証拠には拡大しない。

最新778成功・失敗0・skip0、runner1.499264959秒。`/private/tmp/mon-modal-ready-final-tests-20261011.log` SHA `a457061e94d72f794c81e181e49c176cab8d6657c40852adba2ceda6dea3e501`。型検査／QA終了0、Vite1.45秒／Cargo1分08秒。`/private/tmp/mon-modal-ready-qa-build-20261011.log` SHA `c3710777b467bce704ff94441a6bc9ccb66f67972aaf4dcf84b4310bcce64cbe`。実装時間は未分離。fixture／検査だけの追追加はQA製品sourceへ影響せず、shared target再利用・通常Rust再実行なし。

QA終了後に全11会議の保護対象4組とbootstrapの前節SHA一致、integrity_check ok／FK0行を確認。実データ編集／保存／undo／録音・CI・コミット／Pushなし。正式評価／Windowsは保留、実録音・正式配布等の残件とgoal未完了を維持。

## 2026-10-11 設定間の共通モーダル境界

旧アイコン最終版の冷起動から「再生設定→×→編集オプション」だけでMac AX子要素が欠けることを再現した。閉じたCommonModalのinert追加と再生設定の独立body portal化はそれぞれQAで改善せず、両比較を撤回。メイン画面のOptionsModalと再生設定をSettingsModalScopeの単一native境界へ統合した。本文とreplacementは固定slotへのportalで常時mountし、非active viewはhiddenにする。表示順・親復帰だけを状態ストアで扱い、保存／解析を呼ばない。既存の確認所有者、blocked、restoreTargetと入力を維持し、外部portalのキーはnative境界で処理する。

最初の共通境界QAで欠落順序と2反復の再生→編集→発言設定、分割取消の親／元入口復帰を確認。内側source class重複によるcompact32pxの再適用を削除し、外hostだけに設定種別classを残した。合成の8操作・selectは40px／13px、通常幅720px・横溢れ0、360px viewportでは幅328px・左16px・横溢れ0。親／子設定→確認取消→親復帰を2回実施し、未保存入力保持と実行回数0を確認した。対応CLI driverは登録・構文検査のみで全体実行はしていない。

portalのcommit後に失われたfocusだけを補修するguarded requestAnimationFrameを追加。新しい設定・別native境界・所有された確認・有効な入力のfocusを奪わない。合成の回復画面では初期focusが安全な×に戻るが、最終Macの回復一覧はAX取得可能な一方、初期focusがHTML contentに残った。Escapeで回復入口→外側編集入口へ戻ることは確認済み。Mac初期focusは未解消、全VoiceOver／IME／順序やundo後の個別設定まで完了とはしない。今回はundo／保存／再読込／解析開始を実行していない。

最終QA `/private/tmp/mon-shared-settings-final-qa.pk8emd/MON QA.app`、main SHA `55e76ee170d42e8ea28cf6abfc81a207e4a4a53bba627b8c19c6b8a91c977bed`。ad-hoc deep／strict終了0（正式署名・公証ではない）。冷起動から専用合成会議で再生→編集の全入力AXと×初期focus、さらに再生→編集→00:00:03発言設定を2回反復し、毎回内容取得と発言設定入口へのEscape復帰を確認。最終画像 `/private/tmp/mon-shared-settings-final-native-20261011.png`、`/private/tmp/mon-shared-settings-final-browser-20261011.png`。初回共通境界の画像は寸法修正前なので最終証拠にしない。

最新777成功・失敗0・skip0、runner1.73464775秒。`/private/tmp/mon-shared-settings-frame-tests-20261011.log` SHA `f20aa7225e6a687ae2dd06c51e4eaf738d7e0559486298a8ffb5521411d5c78d`。型検査／QA構築終了0、Vite1.52秒／Cargo1分15秒、`/private/tmp/mon-shared-settings-frame-qa-build-20261011.log` SHA `4758d437c222991ba0ccfa70002c4716dc88519901760c62d1ce65b5037fca18`。実装／手動検証時間は未分離。shared targetを再利用し通常Rust全体は再実行しない。

終了後のreadonly sorted JSONで全11会議の保護対象4組は前節アイコン検証と同じSHA、bootstrapも `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` に一致。integrity_check ok／FK0行。録音・実データ変更・追加取得・CI・コミット／Pushなし。正式評価／Windows・配布／試用等は保留・残件を維持、goal未完了。

## 2026-10-11 補助操作のアイコン統一

ユーザー指定の閉じる／編集オプション／一時停止／先頭へ／再生を閉じる／再生設定／議事録を再読込と、追加・取り消し・前後ページ・その他の再読込・議事録編集・根拠再生を共通SVGへ変更。標準40px、コンパクト操作列32pxの正方形へ中央配置する。各入口にtitleのホバーラベルとaria-labelを残し、SVGはdecorativeとする。既存ref、disabled、保存待ち、イベント、確認境界を維持。保存・削除・採用・録音開始停止・曖昧な復旧操作等は文字を残す。新しい機能やパッケージは追加しない。

CUA合成で再生／一時停止の切替、先頭0秒、再生終了、再生設定の開閉、編集オプションの更新、議事録設定の更新と閉じるを確認。通常幅の入口32px／モーダル40pxとアイコンのみの本文・title／aria名を検査。360×800の議事録画面は横溢れ0、操作は画面内に収まりviewportを解除した。ホバーの実カーソル画像ではなくtitle属性とMac Helpでラベルを確認した。合成画像 `/private/tmp/mon-icon-buttons-transcript-20261011.png`。CLI browser suite全体や全条件付き操作を実行したとはしない。

先行QA `/private/tmp/mon-icon-buttons-qa.yBf2PN/MON QA.app`（main SHA `ca8a3f147352b7ca55e527689dd332d66f5b44ec6f1b38c5762fc685446cbf2c`）で専用合成会議の実音声再生／先頭／再生設定／終了を確認。議事録編集入口と設定の更新／閉じるも実AXで確認。画像 `/private/tmp/mon-icon-buttons-native-20261011.png`、`/private/tmp/mon-icon-buttons-minutes-native-20261011.png`。再生設定の往復・再生終了後に編集オプションを開く順序でWebView AX欠落を再現し、Escでworkspaceと入口focusへ復帰。前項の順序依存問題と同系統の残課題だが、同じ原因と断定せず、全UI／VoiceOver合格にしない。

残る閉じる・解析／生成の一時停止・分割／固定根拠の再生も追反映した最終QAは `/private/tmp/mon-icon-buttons-final-qa.qZLecF/MON QA.app`（main SHA `a8ffd63f8be3a88d68c11847c10ef7cc64d82825be4e67a452ca5e69e567953e`）。両bundleのad-hoc deep／strict終了0。最終版の冷起動から専用会議→発言設定→手動分割で再生／×を画像確認し、保存せず×で親の分割入口、さらに×で元発言設定入口へfocus復帰。画像 `/private/tmp/mon-icon-buttons-split-native-final-20261011.png`、`/private/tmp/mon-icon-buttons-native-final-20261011.png`。最終追加の全条件付きボタンを実機クリックしたとはしない。正式Developer ID署名／公証ではない。

最終フロント769成功・失敗0・skip0、runner1.663643542秒、`/private/tmp/mon-icon-buttons-complete-tests-20261011.log` SHA `0c0eb36b5ee153c3c351dc0e8db5a2eecfd0b67ad891ab628e00877e6366b04e`。型検査／QA構築終了0、Vite1.48秒／Swift0.15秒／Cargo1分10秒、`/private/tmp/mon-icon-buttons-complete-qa-build-20261011.log` SHA `9dd73041e57d0b1adacf5e594121c83181bb12c4a30cd86b35b5bda72bdd2577`。先行QAはCargo1分13秒。実装／手動検証時間は未分離。shared targetを再利用し通常Rust全体は再実行しない。

起動前と最終QA終了後のreadonly sorted JSONで全11会議の下書き＋出力 `16590f765d3723e13188a40fe411eda8e9249e899a51e68e8eac07647969c24a`、版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、設定＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`、bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致。integrity_check ok／FK0行。実入力・保存・解析開始・録音・追加取得・CI・コミット／Pushなし。正式評価／Windows・配布／試用等の残件とgoal未完了を維持。

## 2026-10-11 手動分割の配置とMac分割・一括保存

手動分割は汎用analysis-dialogの980px幅を継承し、4入力が縦に並びすぎていた。幅760px、本文grid16px、時刻／文字数と前半／後半話者を各2列へ整理。600px以下は1列へ戻す。プレビュー内の見出し／本文は8px、保存／取消は8px間隔で右寄せ、取消と音声確認はsecondaryへ統一。入力／select／buttonは40px・13px。保存queue、結果未確認／再試行、分割IDの生成・分割本文完全一致の制約は変更しない。

CUA合成通常幅は760×576.859375px、modal縦scroll0。360×800では幅328px・左右16px・横溢れ0・内側scrollport0、入力は1列。保存後のactiveElementがBODYに落ちることを再現したため、入口buttonを明示focus／保持し、保存で入口行が消えた場合は作成済みの前半articleへfocus復帰する。新IDは既存save／retryと同じIDを参照し、focusのために保存し直さない。取消は親の「発言を手動分割」へ復帰。合成で2→3→undo2発言、連結本文保持、2発言の佐藤への一括確認取消／保存／undoと元の複数話者復元を確認。CLI browser driver全体は未実行。画像 `/private/tmp/mon-split-layout-browser-20261011.png`。

最終QA `/private/tmp/mon-split-focus-qa.lyV50h/MON QA.app`、main SHA `d1d8dd8a2ed4ef5c33bafcf7469fc97375f312bb835dec4c062d56f844fb15ee`、ad-hoc deep／strict終了0。Macの自作会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3` の00:00:00発言を1.920000秒／9文字で保存し、8→9発言・前半行へのfocus復帰・冷起動後保持を確認。SQLは2部分の連結本文一致／境界連続を確認。編集オプションからundoを1回実行して8発言と元payload hashへ復帰。

別の冷起動から00:00:00／00:00:03を選択し、QA話者改名への一括変更確認の取消・再確認・保存、親復帰、冷起動後2発言への割り当て保持を確認。本文は変更せず、編集オプションからundo1回で元payloadへ復帰。lock_versionは13→分割14→取消15→一括16→取消17、最終content_hashは起動前と同じ `8120a0cc0dab61cc767d8dd4a63a7d7ad345e74507545a3ffe185a6d7fbf712c`。全11下書きの履歴・lock_versionが不変との主張ではない。連続undo stackの実装は追加していない。単一TTSの手動編集であり複数実話者の精度合格ではない。Mac画像 `/private/tmp/mon-split-layout-native-20261011.png`。

### 今回新たに再現した順序依存の残課題

Macの「冷起動→編集オプション→直前編集undo→閉じる→発言設定」で見た目のmodalは表示されるがWebView AXが空になる。分割undo後の00:00:00、一括undo後の00:00:03の2回で確認。Esc後はworkspace AXと入口focusへ復帰する。冷起動から発言設定2件→編集オプション→一括確認／保存はAXで操作可能だった。失敗順序の原因をundo自体と断定せず、個別設定と編集オプション間の境界を次の修正対象に残す。全話者編集の順序／VoiceOver合格ではない。未保存入力回復／破棄確認取消・他の比較・全IME／長文／小画面の実機経路も残る。

### 検査時間と保護対象

764件成功・失敗0・skip0、runner1.722252917秒、`/private/tmp/mon-split-layout-final-tests-20261011.log` SHA `7a58ffe33b06cd5e80932598bf0275b54ff4497054bebfbb4ecf85c1399b506f`。型検査／最終QA終了0、Vite1.46秒／Swift0.15秒／Cargo1分12秒、`/private/tmp/mon-split-focus-final-qa-build-20261011.log` SHA `3451c2f02c2d619df25102c9ff6c94ea554a7410e57b7916f190795e36a5c74a`。配置だけの先行QAはCargo1分15秒／Vite1.49秒、最終focus修正は後続bundleで検証。実装／手動検証の工程別時間は未分離。shared targetを再利用し通常Rust全体は再実行しない。

起動前／終了後readonly sorted JSONで他10会議の下書き＋全出力hash `4aaf07870da0306f41797c2782f6caf277b6c3a3effa4af51f1f2041f48734f3`、全11の版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、設定＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`、音声metadata4表 `12d342df56b6fd6b77da41d38a80b84c1990c7017e311e144c9ef7e6adc2d783` が一致。bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致、integrity_check ok／FK0行。全11下書き＋出力hashは想定通り `50217c…` から `16590f765d3723e13188a40fe411eda8e9249e899a51e68e8eac07647969c24a` へ変わる。専用会議の本文payloadは復元されるが操作履歴は保持する。

録音はユーザー指定で今は実施しない。Developer ID／公証は未準備、正式評価／Windows・正式配布／試用は保留または未完了のまま。権限変更・追加取得・CI・コミット／Pushなし、goalをcompleteにしない。

## 2026-10-11 回復記録を編集オプションと同一境界へ統合

前回Macの「編集オプション→回復記録→閉じる」で親まで閉じ、WebView AXが空になる順序を再現していた。TranscriptRecoveryPanelにpresentation用renderPanelを加え、同じcontrollerのentry・replacement descriptor・persistent bodyをOptionsModalへ渡す。回復画面のnative dialogを別に重ねず、親の入力と一覧／プレビュー状態をmountしたまま隠し分ける。閉じる／Escは回復→編集オプション→入口へ一段ずつ戻る。3つの入口はMacのpointer操作でもfocusを明示する。独立使用時のCommonModal fallbackは維持。CAS／一覧の版・hash再照合、結果未確認ガード、破棄の明示確認・resume条件・保存処理は変更しない。

回復一覧は記録行をgridにし本文・操作に12px、view本文16px、前後／回復操作に8pxの間隔を追加。従来pの罫線・paddingを上書きして二重罫線を除き、重複していた下段閉じるは削除しCommonModal共通の閉じるだけに整理。長いIDは折り返し、textareaの枠・角丸・fontをpersistent viewにも適用。

CUA合成通常幅は720px、前後ボタン40px・13px・間隔8px、open dialog1件・内側scrollport0。360×740では幅328px・左右16px・横溢れ0・内側scrollport0、Esc後は回復入口→編集入口へfocus復帰。これらの寸法測定は二重罫線除去前の1回であり、最後の罫線修正後は最終Macの目視を追加した。全preview長文／全入力を測定したとはしない。

最初のQAは `/private/tmp/mon-recovery-owner-qa.ZwRBto/MON QA.app`（main SHA `cc97a51eb9702a40cbb4830a6eb1f81423a7171afa3e6264551625dd2897564a`）。Mac通常GUIで専用合成会議の回復一覧1件／「この記録を見る」／0入力previewをAXで取得でき、閉じるで親を保持し、さらに閉じるで全発言のAXを保持。2回目Escでも各入口focusを復帰。別の回復入力の破棄や実入力再開は実行しない。

目視で二重罫線を発見しCSSだけ追修正した最終QAは `/private/tmp/mon-recovery-spacing-qa.0zGE4V/MON QA.app`（main SHA `a6e33abce5519e613b35d0081f0e995b101bc5ac3891c33a05cb3742dddaa631`）。両QAのad-hoc署名deep／strict検査終了0。最終版でも同じ一覧・プレビュー、閉じるとEscの2反復を実施し、AXと親／workspaceへの入口focus復帰を確認。最終画像 `/private/tmp/mon-recovery-owner-native-20261011.png`。正式Developer ID署名／公証ではない。

並行CUAの合成会議B（解析未採用）では会議操作→人数指定→マイク7→詳細エンジン→人数7保持→会議操作→workspaceのEsc復帰を確認。開始ボタンは未採用でdisabledのまま。会議Aでは追加取消で下書き開始後、00:00:00の15秒発言を7.5秒・5文字で分割保存し、連結本文「次の点検日は来週です。」を保持、2発言→3発言→トーストundoで2発言へ復帰を確認。これらは合成IPCでありMacのfallback／分割や実話者の精度を証明しない。

visible controlsだけの `transcript-recovery-owner-browser-check.js` を登録し、2反復の境界数／各入口focusを検査するdriverを追加。CLI driver自体は未実行。CUA操作とsource契約・構文検査の証拠を区別する。既存の全browser suiteがgreenとの主張ではない。

### 最終検査と不変対象

761件成功・失敗0・skip0、runner1.706343375秒、`/private/tmp/mon-recovery-owner-final-tests-20261011.log` SHA `9e32f231b9775b661c1680d55145f00d72e31c0a6fca5e6c97adf7178f1be47b`。最初757件・後続760件も成功、最終登録後761件。型検査／最終QA終了0、Vite1.55秒／Swift0.15秒／Cargo1分11秒、`/private/tmp/mon-recovery-owner-spacing-qa-build-20261011.log` SHA `e7f4954d8e8458b72353f503780edc2e803d42f202e99dec7c6a5532e2eb54af`。境界修正版QAはCargo1分12秒。実装・手動操作の工程別時間は未分離。shared targetを再利用し通常Rust全体は再実行しない。

最初の起動前と最終終了操作後のreadonly sorted JSON4組で、全11会議の下書き＋出力 `50217c082df595c92893fc4011c552fc52e36e84823d966fde348cd6c91ac3e6`、版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、app_settings＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee` が一致。bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致、integrity_check ok／FK0行（初回QA終了後に検査）。今回Macの本文・設定・回復記録を保存／破棄していない。前回から残る0発言QA手動話者のmetadataは変更せず保持。

回復入力ありの全再開／不可逆確認取消、比較など他子モーダル、Mac分割／一括割当、全VoiceOver／IME／長文／小画面の実機経路は残る。録音・正式品質／Windows・正式配布・先行試用も未完了。goalをcompleteにせず、録音・追加取得・権限変更・CI・コミット／Pushなし。

## 2026-10-11 話者追加の直接導線と実機保存

初回の「＋」が下書きなしではdisabledとなり、無関係な編集から始める必要があった。`requestSpeakerAdd`で同じ解析ID・入力ロック／保存・回復待ちと焦点のガードを維持し、既存のbegin処理が終了した後だけ追加画面を開く。画面を開いただけでは話者を公開しない。発言内の追加も同じ導線を使用し、対象発言の存在を再確認する。小画面は話者drawerをdismissして単一の追加modalへ移り、非表示になった「＋」ではなく一覧入口へ取消focusを戻す。

入力1件の追加画面を980pxから520pxへ縮小し、本文grid gap16px、label gap8px、左右余白を確保。下段キャンセルをsecondaryへ変更。CUA合成通常幅で520px、360×740で328px／横溢れなし／開いているdialog1件、通常ボタンは40px・13px。小画面取消後のactiveElementは「話者一覧を開く」。一度目の測定は再読込前の旧CSSで980pxだったため、その結果は最終表示の合格に用いない。検証タブを再読込して520pxを確認。画像 `/private/tmp/mon-speaker-add-ui-20261011.png`。

合成画面では初回の直接追加→QA補助を作成→QA補助改名へ変更→発言へ追加割り当て→田中へ統合保存→トーストから取り消しを実施。本文2件・時刻は維持。CLIの古い話者browser driver全行程は未実行。

Macは最終bundle `/private/tmp/mon-speaker-add-qa.ofH6UE/MON QA.app`、main SHA `766a6715274d0078effbce47ac95dd429d727be91b293794f2d3d133fe44d9b5`、ad-hoc署名のdeep／strict検証終了0。自作合成会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3` の既存下書きだけで、追加→QA話者改名へ名前保存→00:00:00発言へのcheckbox割り当て→QA読み上げへ統合確認・保存→統合取消を実施。親／子の全入力AXをこの行程で確認。Cmd＋Q／再起動後もQA話者改名と割り当てが保持された。単一TTSの手動追加であり複数実話者の精度合格ではない。

既存undoは最後に保存された操作を反転し、そのundoをもう一度反転できる仕様（既存Rust試験も往復を要求）であり、連続stack undoではない。取り消し2回は過去の名前／追加まで遡らないことをSQLで確認し、それ以上繰り返さず通常checkbox操作で検証用割り当てを解除した。最終のlock_versionは13、元の4訂正文・元speaker_idsは維持し、0発言の手動話者 `manual:391df3bf-fae8-4cf4-9187-924ba669d39e`／QA話者改名を検証用に残した。元payload完全一致／全11下書き不変とはしない。新しいundo機能は実装していない。

### 新たに再現した残課題

Mac「編集オプション→旧下書きを含む回復記録」で子の見た目は表示されるがAXは閉じるボタンだけとなり、閉じると親も閉じてWebView AXが空になる。再起動で復帰した。別native境界の回復記録は修正対象であり、全設定／全操作／全VoiceOver完了ではない。手動分割、一括割り当て全経路、未採用fallbackも今回未実行。閉じた状態から即再起動する1回は終了処理と競合して起動失敗、別callの再起動は成功した。

### 検査・保護対象

757件成功・失敗0・skip0、1.530421625秒、`/private/tmp/mon-speaker-add-tests-20261011.log`。型検査／QA build終了0、Vite1.47秒／Cargo1分10秒、`/private/tmp/mon-speaker-add-qa-build-20261011.log`。共有targetを再利用し、通常Rust全体は再実行しない。実装／手動操作の個別時間は未分離、工程小計を総所要時間にしない。

起動前／終了操作後のreadonly sorted JSON集計で、他10会議のworking_drafts＋全export_records SHA `4aaf07870da0306f41797c2782f6caf277b6c3a3effa4af51f1f2041f48734f3` は一致。全11の版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、app_settings＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`、bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は一致。全11下書き＋出力は検証用変更により `888868a912b99090c062f9607f5f3a6cf5057d1930359fefbfe4af235fa4c4bb` → `50217c082df595c92893fc4011c552fc52e36e84823d966fde348cd6c91ac3e6`。整合性ok／FK0行。録音・AI開始・新規取得・正式署名／公証・CI・コミット／Pushなし、goalは未完了。

## 2026-10-11 録音準備の実機フォーカスと選択欄

最新余白QAの非録音キーボード確認で、Cmd＋Fは一覧の検索欄へ移ったが、Cmd＋Nで準備が表示されても会議名へ焦点が移らずHTML rootに残る問題を再現した。画像でも入力枠のfocusがないことを確認。この旧QAを合格扱いにしない。画面遷移直後のanimation frameだけに依存せず、RecordingWorkspace自身がmountと入力ロック解除後に同じ準備IDへ一度だけfocusするよう変更。入力中・別の欄・説明／dialogを開いた利用者の焦点を奪わず、録音開始・入力選択・保存条件は変えない。

同じ実画面ではマイクのnative selectだけが周囲の40px入力／ボタンより低く描画されていた。既存のtheme／modal／audio selectと同じappearance・背景・chevron・角丸へ加え、40px／13px／20px・右32pxを保持。forced-colorsではnative appearanceへ戻す既存条件も維持した。これはHTML selectのままで、独自の選択機能や権限は追加しない。

source契約2件を追加、最終フロント736成功・失敗／skip0（runner1.717秒）。途中の735成功／1失敗は、共有selectルールへcapture欄を加えた後の古いselector期待によるもの。対象を含む期待へ更新し、寸法・native無効化・forced-colors条件を削除せず再成功した。単体契約だけでfocus競合の動作合格とはしない。型検査／build成功（Vite1.46秒）、diff検査成功。

先行focus QAはCargo1分10秒・Vite1.55秒で構築したが、後から加えたselect修正を含む最終QAは別にCargo1分09秒・Vite1.46秒で構築した。保持先 `/private/tmp/mon-preparation-focus-final-qa.EFYgac/MON QA.app`、main SHA `d8aed126d2a116f44bbf9cb422adb3d2b9a9998a455fd685b795973ef4b7762b`。全コピー一致・strict deepアドホック署名成功。署名・公証済み配布物ではない。既存の専用build targetを再利用し、新しい大容量targetは作らない。

最終Mac実WebViewでCmd＋Nを2回実行し、どちらも会議名がnative AXのfocused element、画像でも2px focus枠とcaretが見える。Tabでマイク再確認→マイクselectへ到達し、SpaceでOS選択を展開、Escで同じマイクのまま閉じた。次のTab→SpaceでPC取得範囲のinfoを表示し、安全な閉じるへ焦点が移り、Escで元infoへ戻った。会議一覧へ戻りCmd＋Fで検索欄の焦点を確認。selectの高さも画像で周囲の入力／ボタンと揃った。録音・5秒テスト・権限変更・マイク変更・会議作成・本文編集・解析・保存は行わない。公開可能な既定名だけの画面 `/private/tmp/mon-preparation-focus-native-final-20261011.png` を保持した。

Cmd-QとQA本体不在を確認後、読み取り照合：integrity_check=ok／外部キー0行／会議8。全下書き＋出力記録SHA `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、全解析＋議事録版（候補を含む）SHA `8f25b0b3025006aa30e2a68a309ca8de75346f0ace9fc94d71b1797e9666d50c`、会議タイトル／現在・直前head SHA `6a04f298df7bfa0f91ff28c1f097085bd72acb8b871fff48b5548c4d83b36d78`、bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は今回起動前と全て一致。全版の比較は以前の確定版だけの比較と集計範囲を区別する。合成ブラウザーのクリックからのfocusも確認し、補助操作を閉じ通常文字起こしを保持した。正式評価・Windows、全キーボード／IME／VoiceOver・実録音等は残る。CI・コミット／Pushなし。

## 2026-10-11 横断点検と本文選択

最終再確認では1280×720と960×640の合成画面で編集オプションの可視9操作／selectが40px・13px・行高20px、dialog幅720px・横溢れ0・内側scrollport0を確認。960pxの見出し／状態／案内／入口の中心差は0.004px未満、会議タブと会議操作の縦中心は165.6875pxで一致。今回の再確認では追加の製品変更・保存を行わず、Escで入口へ戻してviewportを解除、通常文字起こしを確認タブに保持した。最終合成画像 `/private/tmp/mon-ui-spacing-audit-final-20261011.png`。

通常1280pxと狭幅360／320pxで、会議一覧、文字起こし、議事録設定、表示・モデル・データ・用語設定、通知を追点検した。合成の設定画面にはfixture未対応の保存先確認エラーがあるため、実際の保存先変更の合格とはしない。登録前の用語の有効チェックと削除操作は縦中心一致、入力は40px／13px。議事録設定のテキスト操作・selectも40px／13pxで、情報アイコンは別のコンパクト寸法を維持する。

文字起こしの編集オプションには絶対配置と固定118pxの予約余白が残っていた。入口だけを既存OptionsModalのtriggerHostで見出し操作列へ移し、12pxの列間・8pxの折返し間隔と中央揃えに変更。本文の固定モーダルhostと保存・回復・確認の所有者は変えない。1280pxでは見出し・保存済み・案内・入口の縦中心230.6875pxが一致。360pxでは見出し／保存状態と、案内／入口の2行に分かれ、同じ行の縦中心が一致。320pxでも折返しと横溢れ0。320／1280pxの編集オプションは可視9操作・selectが40px／13px／20px、横溢れ0・内側scrollport0、Escで入口へ焦点復帰。

複数行の合成通知は本文・状態アイコン・説明・閉じるの縦中心117.796875pxが一致。通知host／本文のoverflowはvisibleで、cardのscrollHeightとclientHeightはともに66px、通知にスクロール領域はない。補助タブを閉じviewportを解除し、1280×720の合成画像 `/private/tmp/mon-global-spacing-final-20261011.png` と確認タブを保持した。今回の点検で全条件付き画面・全VoiceOver／IMEまで合格したとはしない。

Macでは本文のドラッグが直ちに編集欄を開く問題を再現。選択状態の判定だけを追加したネイティブbutton版も実機では改善せず、その結果を不合格として扱った。現在は本文を文字選択可能なspan＋button roleへ変更し、実際の非空選択が本文にかかるクリックでは編集を開かない。Enter／Space、非活性状態、クリック編集、アクセシブルな名前は維持。QA `/private/tmp/mon-body-selection-span-qa.H6GW5l/MON QA.app`（本体SHA256 `0c13be2d7d7d94453f17e78c2e6eb358f2618efec27f7aac93b359b0c7e243fc`）はコピー一致・strict deepアドホック署名を確認。実WebViewで本文のドラッグ後に選択が残り編集欄が開かないこと、EditメニューのCopyが有効、通常クリックとTab→Enter／Spaceで編集が開くことを確認した。コピー実行・文字入力・確定はしない。各編集はEscで取消しCmd-Q後QA本体不在を確認、保護対象4組SHAとDB整合性は一致。実機の私的本文の画像は外へ掲載しない。

最新フロント734件成功・失敗／skip0（runner1.603秒）、型検査／本番build成功（Vite1.45秒）、diff検査成功。見出し操作列変更後の最終Mac QAはCargo1分08秒／Vite1.45秒で構築成功。`/private/tmp/mon-global-spacing-qa.PwTjOP/MON QA.app`（本体SHA256 `f62fd896871ecdc329d4162a167302396b9bcf1d8202c4aa42f1ff57be51d2c0`）のコピー一致・strict deepアドホック署名を確認し、実WebViewの保存状態付き見出し操作列と編集設定の間隔・中央揃えを画像確認した。モーダルの全操作のnative AX列挙、閉じる初期焦点、Escで移設した入口への焦点復帰を確認。録音・再生・解析・文字入力・確定は行わない。終了後保護対象4組SHA一致・整合性ok／外部キー0行。内部実機画像は `/private/tmp/mon-global-spacing-heading-native-20261011.png` と `/private/tmp/mon-global-spacing-options-native-20261011.png`、私的本文を含むため外へ掲載しない。CI・コミット／Pushなし。

## 2026-10-11 操作列の狭幅追点検

会議一覧、録音準備、設定、文字起こし、発言設定と編集オプションを合成ブラウザーで再点検。360px幅の会議詳細では「議事録」タブの右端と絶対配置された「会議操作」の左端がともに254pxで間隔0、縦中心にも4.5pxの差があった。

会議操作の入口だけを既存OptionsModalのtriggerHostでタブ外の共通操作列へ配置した。設定本文の固定host、保存・確認・録音・解析の条件は変更しない。操作列は中央揃え・列間16px・折返し対応にし、再生dockの高さ計算も折返しを含む操作列全体をResizeObserverで計測する。360pxではタブと操作の縦中心181.28125pxが一致、議事録と操作の間28px。320pxでは8pxの行間を保って別行に折返し、ページ横溢れ0。1280pxでも縦中心165.6875pxが一致・横溢れ0。

360pxの編集オプションは可視8操作／選択欄がすべて高さ40px・文字13px・行高20px、dialog横溢れ0・内側scrollport0。会議操作と編集オプションのEsc取消は各入口へ焦点復帰し、録音・再生・解析・保存は実行しない。viewport overrideを解除し、通常文字起こしの合成画像 `/private/tmp/mon-ui-spacing-navigation-final-20261011.png` と確認タブを保持した。

限定65件と最終フロント728件成功（失敗・skip0、全体runner1.791秒）、型検査・本番build成功（Vite1.65秒）、git diff --check成功。既存JS chunk警告は残る。この追修正のMac QA再構築は未実施。CI・コミット／Pushなし。全条件付き画面・全VoiceOver／IMEの検証完了には拡大しない。

## 2026-10-11 固定hostの開く順序と実機反復確認

前節で残った反復時のAX欠落に対し、開くことをキューが許可した直後・showModal前に、閉じたOptionsModalの固定hostだけをbody末尾へ揃えた。hostと入力ノードは再mountせず、既に開いた境界・body以外のhostは移動しない。既存のnative順序追跡・busy・確認所有者・実行ガードは維持。ノード同一性と除外条件、キュー許可→host調整→showModalの順序を回帰で検査した。OS原因を確定した修正ではない。

最新QA `/private/tmp/mon-modal-host-order-qa.jWksuc/MON QA.app` はCargo表示1分17秒／Vite1.44秒、本体SHA256 `8d5aff0dc37e0737dd24ce6e8f5aa1c47a1eca67eeec26ff4b7deca3dde472fa`。コピー全一致とstrict deepアドホック署名を確認。最終フロント727件成功／失敗・skip0（runner1.398秒、実経過1.456秒）。CI・コミット／Pushなし。

Mac冷起動から文字起こし編集オプション→Esc→議事録設定→候補#3比較→採用前確認を取消→比較を取消→設定を閉じ→文字起こし編集オプションを再表示する経路を2巡した。各回で本文と操作のnative AX列挙、初期の安全な閉じる焦点、段階ごとの入口への焦点復帰を確認。2巡目では設定内「編集の管理」の子dialogも開閉し、親設定の内容・入口焦点が戻った。以前のrootのみになる症状はこの限定された反復では再現しない。実機の編集オプションの余白・隣接操作の上下端も画像確認。全VoiceOver・IME・全モーダル順序の合格には拡大しない。採用・生成・録音・再生・保存は今回実行しない。

Cmd-Q後、対象QA本体不在を確認してからreadonly DBを検査。整合性ok／外部キー0行／会議8／ジョブ成功10・失敗1。下書き＋出力履歴・確定結果・会議タイトルと版参照・bootstrapの4組SHAは前節の値と全て一致。最終の公開用合成画面 `/private/tmp/mon-whole-spacing-final-20261011.png` は1280×720／ページ横溢れ0、確認操作を閉じた通常文字起こし。実機画像 `/private/tmp/mon-host-order-options-native-20261011.png` は私的本文の背景を含み内部証拠のみ。以下のAX残課題の記載は、この追確認より前の履歴。

## 2026-10-11 安定した設定hostと反復時の残課題

切替後のAX欠落について、明示role=dialogのみ、および実際のshowModal後だけaria-modalを付けclose時に外す変更をそれぞれアドホックQAで比較したが、文字起こしオプション開閉→議事録設定の欠落は再現した。両案は製品ソースから戻し、恒久ARIA modal指定・タイマーによる焦点移動は加えていない。

OptionsModalのnative境界をhiddenになるtabpanelの外へ、body直下の固定hostに配置した。入口は元の場所、設定入力は開閉・タブ切替のたびに再mount／移動せず、hostのdisplay:contentsで既存の用途別CSSを保持する。native showModal、確認所有者、busy・保存キュー・実行条件のガードを変更しない。合成UIの比較→復元前確認→取消はdialog1件、要求0、元入力と各入口焦点を保持。設定内「編集の管理」の子dialog開閉では親設定が残り、閉じた子の入口へ戻る。通常幅の3操作は同じ上端157.297px／高さ40px／文字13px、360×740は左16px／幅328px／横溢れ0／内側scrollport0。合成画像 `/private/tmp/mon-whole-spacing-confirmed-20261011.png` は私的本文なし。

最新QA `/private/tmp/mon-stable-modal-host-qa.U3AV9h/MON QA.app` はCargo1分18秒、Vite1.46秒、本体SHA256 `7c9031a29cb5da2ee89f3f7c505b988b027013aa11c7d58f7cfd0b91c025d8ac`、strict deepアドホック署名と全コピー一致。フロント725件成功（失敗／skip0、runner1.542秒）。録音・再生・生成・採用・編集保存はこの追検証で実行しない。

実機では冷起動→文字起こし編集オプション→Esc→議事録設定の本文・操作・初期閉じる焦点が列挙され、従来欠落したこの経路は改善。続く候補#3比較→採用前確認→Esc→比較取消→設定では各本文・安全な入口・焦点復帰を列挙し、入力と現在版を保持した。ただしさらに設定を閉じ文字起こしへ戻り、編集オプションをもう一度開くとAXがHTML rootだけになる反復時の欠落が残る。画像の余白・配置・閉じる焦点は正常、Escで本文と編集オプション入口焦点へ復帰。内部画像 `/private/tmp/mon-stable-host-repeated-modal-ax-20261011.png` は私的本文を含み公開しない。初回経路の改善だけを全VoiceOver／全順序合格やOS原因の確定に拡大しない。

Cmd-Qと対象本体不在を確認後、readonly DB整合性ok／外部キー0行／会議8／ジョブ成功10・失敗1。下書き＋出力履歴／確定結果／会議タイトルと版参照／bootstrapの4組SHAは通常v4生成後の値と全て一致。CI・コミット／Pushなし。最新v4限定Rustは86成功／3ignoredで終了、全Rustと計画全体の残件は維持する。以下の各QA記録はこの追確認前の履歴。

## 2026-10-11 議事録比較の単一native境界（実機追検証中）

追確認では通常v4生成が成功し、候補#3／2項目→比較→実行前確認→Esc取消→比較取消→設定入口への焦点復帰もnative AXで確認した。採用せず現在版#1を保持。その後、文字起こしタブ→編集オプション開閉→議事録タブ→設定再表示で、設定の画像表示は正常でもnative AXに本文・操作がない別経路を再現した。Tabでも変化せず、Escで閉じると背景の内容と設定入口焦点が戻る。従って単一境界の冷起動比較改善を全モーダル合格に拡大しない。Cmd-Q・対象本体不在を確認した後、readonly DB整合性・外部キー・保護対象4組SHAの一致を確認。通常v4と最終限定Rust86成功／3ignoredの終了証拠は実AI検証記録に追記した。

冷起動した設定から候補比較へ進むとnative AXから比較本文・操作が欠落する再現に対し、議事録比較を別dialogで重ねず、設定の同じCommonModal内のreplacementへ切り替えた。元設定の入力・所有者・保存キューはhiddenのままmountを保持。確認サービスは引き続き実際の最前面dialogを所有し、採用／復元の版・hash・下書き・結果不明ガードは変更していない。表示中の名前をreplacement／確認の見出しへ切り替え、元設定のaria-labelを残して新しい見出し名を上書きしない。取消時は元のscrollと入口へ、採用で入口が消えた場合は設定見出しへ戻る。読込中に比較が表示された場合のheading焦点は、解除後に安全な閉じる操作へ移し、利用者が別の欄へ移った焦点は奪わない。

合成の実UIで、設定→直前版比較→実行前確認→取消はすべてopen dialog1件。元入力 `/fixture/llama-completion` を保持、比較取消後は元の入口へ、確認取消後は比較の実行入口へ復帰した。取消時は復元／採用要求0。明示復元1回と、別の新規合成候補の明示採用1回でheadだけが期待する版へ進み、保存済み6ブロック／2項目／2根拠・生成原文・初期編集JSONを保持。採用入口が消えた後は設定見出しへ復帰した。360×740比較は左16px／幅328px／dialog横溢れ0。これは条件付き状態を含む全ブラウザーdriverの再実行ではなく、上記の選択された実操作とsource contractの証拠である。

フロント725件成功（失敗・skip0、runner1.629秒）、型検査／本番build成功（Vite1.50秒）。最新QA `/private/tmp/mon-v4-review-qa-artifact.XU5GMZ/MON QA.app` はCargo表示2分24秒で構築、identifier `jp.mon.desktop.qa`、本体arm64、strict deepアドホック署名・生成元との全コピー一致を確認。本体SHA256 `18702858aa2b0beec08a210918edc4183e056fa1b6cb17b8f1adfcb26204c20a`。公証なし、配布署名完了ではない。

実機の冷起動→既存QA会議→設定→旧候補#2を開き、今回native AXは比較本文／根拠／実採用入口／閉じる操作を列挙し、閉じる操作へ焦点があった。実行前の採用確認も正しい新しい名前・対象・キャンセル・実行入口が列挙された。確認をEscで取消し、さらにEscで設定へ戻すと候補確認入口へ焦点が復帰し、元設定の値と旧6項目候補が表示された。採用はしていない。この再現経路は改善したが、全モーダル順序・実VoiceOver／IME・新生成完了後の反復操作まで合格としない。実機画像は私的本文を含むため内部証拠のみ、合成画像 `/private/tmp/mon-single-review-20261011.png` を公開用証拠とする。既存モデルで新規v4生成を開始し、完了後の候補・終了後の保存データ照合は追検証中。CI・コミット／Pushなし。

## 2026-10-11 再横断点検：発言メニューと狭幅プレーヤー

文字起こし・再生設定・表示設定・議事録本文／設定・複数行通知を合成ブラウザーで確認。追加で発言メニューの絶対配置（メタ情報との中心差1.5px、右側余白0）と、360px幅でプレーヤーの説明が固定操作に押されて一文字ずつ折り返す状態を見つけた。

発言行を2列gridにし、メタ情報とメニューを同じ行に配置、列間12px／右内側8pxを確保。通常幅の2行は中心差0、横溢れなし。狭幅の複数話者行ではメニューを最初のメタ情報行に揃え、時刻や補足は下へ折り返す。本文は列全体を使い、編集・選択・音声の処理は変更していない。共通テキスト操作にも最大幅と長い語の折り返しを設定した。

760px以下ではプレーヤーの説明を独立行にして操作を折り返す。360×740の実表示はプレーヤー高107px、説明幅240px、発言本文の表示高223px、ページ／本文の横溢れ0。設定モーダルにはmax-width720pxを明示し、共通確認画面の560px上限による意図しない圧縮を解消。960×640で議事録設定の幅720px、隣接3操作すべて40px／13px、横溢れ0を確認した。

複数行トーストの本文・状態アイコン・info・閉じる操作の縦中心はすべて37.796875px、横溢れ0、host overflow:visible。文字起こしの発言設定は狭幅でも開閉でき、操作とモーダル内本文の余白を画像確認した。補助通知タブを閉じ、viewportを解除し、通常の文字起こしを残した。確認画像 `/private/tmp/mon-spacing-cross-screen-20261011.png` は合成本文のみ。

フロント722件成功・失敗／skip0（runner1.982秒）、型検査＋本番build成功（Vite1.83秒）。大きなJS chunk警告は継続。今回の証拠はブラウザーの上記状態に限定し、最新変更のMac実機検証・全条件付き状態の確認を完了扱いにしない。実データの保存・録音・生成・採用・CI・コミット／Pushなし。

## 2026-10-11 仕上げ後の横断確認

既存の余白修正を保持して、合成ブラウザーの会議一覧・文字起こし・全体設定・中断削除管理を再確認した。この追確認では製品コードの追加変更は行わず、実録音・解析・取得・保存・削除も実行していない。

960×640では一覧の検索欄と再読込ボタンがともに高さ40px／文字13px／同じ上端。話者見出しと全発言フィルター間16px、発言行の話者・時刻・複数話者バッジの縦中心差は0.004px未満だった。管理の5カードは内側padding24px／群間16px、見出しとinfoの中心差0.004px未満、外側dialogの横溢れ0／内側独立スクロール0。編集オプションの可視8操作・selectはすべて40px／13px、各区画内gap12px、横溢れ0／内側独立スクロール0を再確認した。隠れた回復モーダルのheight0の操作は可視操作数に含めない。

全フロント719件成功・失敗／skip0（runner1.528秒、実経過1.72秒）、型検査＋本番build成功5.94秒（Vite1.50秒）。大きなJS chunk警告は残る。今回の追加証拠は合成ブラウザーの上記状態に限定し、先行Mac QAのAX残課題や計画全体の完了を証明しない。Rust再構築・CI・コミット／Pushなし。

## 2026-10-11 全体余白の仕上げ再点検

会議一覧／会議操作／文字起こし／議事録／再生設定を再点検。再生設定のlabel margin8pxが外側gap16pxと重なり、間隔24／32pxとなる箇所をmargin0・ラベル内8pxへ整理し、実測で各群の間16pxを確認した。復元会議の案内に専用flex行を設け、文と操作の間10px12px・中央揃え・折返しを確保。復元案内の今回の証拠はsource contractで、実際の復元操作は繰り返していない。

議事録のページ移動は専用gridで群間12px・本文から16px、子margin0へ整理。番号欄に残るpadding6pxを隣接操作と同じ9px12pxへ揃えた。長い合成議事録で入力／移動／前後ボタン40px・文字13px、入力と移動ボタン上端一致、群間12pxを実測。既存の番号検査・ページ遷移条件は変更しない。モーダル内card-headのh2／h3／h4 marginも0へ揃え、隣接操作への上下のずれを防ぐ。

取り込み型モーダルの余白gridはnot([hidden])へ限定し、所有された変更確認へ切り替えた際に元本文を再表示しないよう修正。合成UI `action-confirmation-browser.html?layout=import` の実共通モーダルでopen1件、旧本文hidden=true／display:none／height0を確認。取消後は元本文grid・入力保持・候補採用入口へ焦点復帰。実ファイルの変更はしていない。テスト用window状態の直接取得はread-only DOM環境でできず、実行要求数の検証には数えない。

960×640の編集オプションは可視8操作／selectすべて40px／13px、横溢れ0。viewportを解除し補助2タブを閉じ、通常の文字起こし編集オプションを保持。合成画像 `/private/tmp/mon-ui-spacing-final-20261011.png`（私的会議本文なし）。限定source38件成功、全719件成功・失敗／skip0（runner1.366秒、実経過1.54秒）、型検査＋本番build成功5.55秒（Vite1.43秒）、diff検査・cargo fmt成功。JS chunk警告は継続。全条件付き状態・全狭幅・実IME／VoiceOverの保証ではない。CI・コミット／Pushなし。

最終Mac QAは構築86.51秒、`/private/tmp/mon-ui-spacing-final-qa.Ur7hX9/MON QA.app`（本体SHA256 `61ab62a47f96d450f34af5c246a42c9ffde8c4c66dec4b8e9c90927a7935d0db`）。構築元との全コピー一致・strict deepアドホック署名・本体arm64を確認。実WebViewで8会議、保存済み文字起こし2発言、現在議事録#1と編集マーカーを保持し、再生設定の群間隔、編集オプションの保存／回復操作3件の行揃え、議事録設定3操作の配置を画像確認した。再生設定／編集オプション／議事録設定はEscで各入口へ焦点復帰。議事録設定では今回もnative AXが本文を列挙しない観測があり、Tab後も変わらなかった。配置の視覚確認とアクセシビリティの合格は区別し、AX問題を未解決で残す。画像3件はQA保持先の内部証拠で、背景に私的本文があるため外へ掲載しない。録音・再生・保存・解析・候補採用はこのUI確認で実行していない。

Cmd-Q後の対象本体・構築プロセス不在を確認してreadonly DBを再検査。整合性ok／外部キー0行／会議8／ジョブ成功9・失敗1、下書き＋出力履歴／確定解析＋議事録／会議タイトルと版参照／bootstrapのSHA256は先行通常v3生成後と全て一致（各値は実AI検証記録に保存）。専用構築target `/private/tmp/mon-minutes-v3-qa-build.z7ktoV` 約1.3GiBのみ回収し、静的QA・会議DB・モデル・共有Rust cache8730.5MiBを保持。削除した構築物は再構築可能。

## 2026-10-11 全体余白の追加監査：管理・取得確認

会議一覧・会議操作・文字起こし通常表示／空状態／編集オプション・解析候補プレビュー・設定・中断削除管理・モデル取得記録／取得内容確認・議事録本文／設定を合成ブラウザーで点検した。既存の未コミット変更を保持し、今回の製品変更はCSSのみ。保存・削除・再生・取得の条件やIPCは変更しない。

追加で確認した問題は管理の「中断したデータ操作の確認」見出しと直後の「要確認一覧を再読込」の間隔0px。管理カードの実際の子要素をgrid gap16pxにし、段落の有無に依存しない配置へ変更。カード外marginの重複と空リストの余白も除去した。修正後、見出しと直後の操作は16px、管理の5操作はすべて高さ40px・文字13px。

モデル取得記録のリストは既存max-height360px／overflow:autoを解除し、モーダルが唯一のスクロール境界となるよう変更。取得内容確認の本文にはgap16px、複数素材の間に12pxを設け、確認文と直後のボタンが接しないようにした。合成33bytesの定義をUIから入力・確認した結果、確認文とボタン間16px・ボタン40px・内側スクロール0を実測。取得・コピーは実行せずキャンセルし、共有準備は待機へ復帰した。取得記録は空の合成一覧でoverflow:visible／max-height:noneを確認しており、多数の記録がある状態の実操作検証ではない。

既存修正も再確認：候補プレビューの再生は32pxアイコンでmetadataとの間隔12px。文字起こし空状態はpadding24px32px。960×640の実測画面で編集オプションの可視ボタンとselectはすべて40px／13pxで折返しと余白を維持。議事録設定の3操作は上端157.297px／40px／13pxで一致、数値6入力も40px／13px、内側スクロール0。最終viewport overrideを解除、補助2タブを閉じ、文字起こしの合成確認画面だけ保持。画像 `/private/tmp/mon-ui-whole-spacing-20261011.png` に私的会議本文は含まない。

追加のsource回帰2件を含むフロント714件成功・失敗／skip0（runner1.630秒）、型検査＋本番ビルド成功（Vite3.46秒、全build経過未計測）、git diff --check成功。既存JS chunk警告は残る。今回の追加CSSでMac QA再構築・Rustテスト・CI・コミット／Pushは行っていない。全条件付き状態や全狭幅／IME／VoiceOverの検証完了とはしない。

## 2026-10-11 最終追加点検：議事録設定とMac反映

検索／用語／人数指定／詳細書式の追加分をまとめてMac QAへ反映。最初の実機確認で、議事録設定のポータル内「編集の管理」が単独行となり、再読込・再生成ともボタン間隔がなく接している箇所を発見した。3入口を同じflex操作列（折返し・stretch・gap10px）へまとめ、ポータルhostはdisplay:contentsで操作列に参加させた。内側の設定は項目間16px・ラベル間8px、セクション間20px、候補／出力履歴12pxへ整理し、重複marginを除いた。既存controller・保存／確認／再生成条件・IPCは変更しない。

source回帰は修正前失敗→修正後成功、限定31件成功。最終フロント712件成功／失敗・skip0（runner1.480秒）。合成ブラウザーでは3操作とも高さ40px・文字13px・行高20px、上端157.297px・間隔10px、設定9入力も40px／13px／20px、横溢れ0を確認。合成画像 `/private/tmp/mon-minutes-settings-spacing-synthetic-final.png`。全条件付き状態／全狭幅の合格とはしない。

初回QA構築146.06秒、追加修正後の構築86.68秒。いずれも型検査・本番ビルド成功、JS chunk警告は残る。最終保持先 `/private/tmp/mon-ui-wal-final-qa.erNHFS/MON QA.app`（本体SHA256 `639c80c0abc74a77c528dcddd4d72dd7b344286cb502700472fd9e9f5f43fb36`）。構築元との全コピー一致・strict deepアドホック署名・識別子jp.mon.desktop.qa・全3実行ファイルarm64を確認。Developer ID／公証の証拠ではない。

Mac実WebViewで8会議保持、保存済み文字起こし2発言、保存済み議事録#1と既存編集マーカーを確認。設定の3操作と項目間隔を画像確認し、再読込後も表示保持、Escで設定入口へ焦点復帰。Cmd-Q→本体不在確認→同じ最終QA冷起動でも8会議と同じ文字起こし／議事録の保存表示を確認して終了した。再生成・録音・設定保存・候補採用は実行していない。最初の比較QAではこの設定のAXツリーが空になる観測があったが、最終QAでは設定内容と各操作がAXへ列挙された。全アクセシビリティ状態の解消とはしない。実機画像は私的本文が背景にあるため内部証拠 `/private/tmp/mon-ui-wal-final-qa.erNHFS/minutes-settings-native-final.png` として扱う。

終了／本体不在を確認してからreadonly DBを検査：integrity_check=ok、foreign_key_check0行、会議8、ジョブ成功8／失敗1、確定議事録2。下書き＋出力履歴、確定解析＋確定議事録、会議のタイトル＋現在／直前版参照の3組の全行JSON SHA256はそれぞれ `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、`8a1d80c720c6f32f27282231a38f21dd95c744d29213eae49d14a34754cdb620`、`6a04f298df7bfa0f91ff28c1f097085bd72acb8b871fff48b5548c4d83b36d78` で起動前と一致。bootstrap SHA256も `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` を保持。検査側の最初のジョブ集計は存在しないstatus列を参照して失敗し、実スキーマのstate列で訂正・成功した。

最新DB修正も同梱しており、この限定された読込み／再起動では異常終了の再発なし。過去SIGBUSの正確な因果や長時間耐久、通常議事録生成の不完全JSON問題、最新Rust全体／doc testは未解決・未検証のまま。終了済みの専用構築target約1.3GiBのみ回収し、最終QA・比較QA・会議DB・モデル・共有Rust cacheは保持（構築targetは再構築で復元可能）。CI・コミット／Pushなし。

## 2026-10-11 横断再点検：検索欄と詳細書式

会議一覧／会議操作、文字起こしの通常表示／編集オプション／発言設定、設定の表示／モデル取り込み／保存の初期表示、議事録の空状態と生成済み合成紙面／編集／詳細書式をCUAで再点検。既存の未コミット変更と保存条件を保持した。

追加で発見した不揃いは会議一覧の検索欄。`:root input`のfont shorthandがclassだけの指定より強く、検索欄だけ15px／高さ45.5px、隣の再読込は13px／40pxで上下端がずれていた。fontの優先順位を明示して双方を40px／13px／行高20px／padding9px12pxに統一。修正後の実測で双方の上端295.289px・高さ40pxが一致した。文字起こしの検索も40px／13px／20px／左右12pxへ統一（実測一致）。

詳細書式は入れ子の説明とtoolbarが外側gridの間隔を受けず、段落・toolbarのmarginに依存し、折返しボタン間隔が6pxだった。内側を16px gapのgrid、段落／toolbar margin0、ボタンgap10pxへ整理。CUA実測は説明と操作間16px、全15ボタン40px・13px／20px、横溢れ0。議事録の主操作は32px／12pxで同じ上端・縦中心、見出し／infoの中心差0.004px以内を確認。書式・保存・削除を実行せず編集を終了し、確認用議事録タブを閉じて既存の文字起こしタブを保持した。

追加source回帰2件はそれぞれ修正前失敗を確認し修正後成功、限定30件成功。最終フロント711件成功／失敗・skip0（runner1.520秒）、型検査＋本番ビルド成功（Vite1.57秒、全build経過未計測）、git diff --check成功。JS chunk警告は残る。画像は合成内容の `/private/tmp/mon-ui-spacing-search-audit.png`、`/private/tmp/mon-ui-spacing-format-audit.png`。Mac最新反映・狭幅・保存管理の全条件付き状態は未検証で、全UI状態の合格とはしない。CI・コミット／Push・録音・モデル取得なし。

## 2026-10-11 横断点検の再確認：用語編集の文字サイズ

文字起こしの通常表示、編集オプション、設定の表示／ローカルAI／保存／共通用語、議事録の空状態を合成IPCのブラウザーで再点検。編集オプションの一行入力と隣接ボタンは高さ40px・文字13px・行高20px・上端一致。設定4見出しとinfoの縦中心差は0.004px以内。議事録の左右余白32px・操作列の縦中心一致を確認した。

共通用語に空欄の合成項目を追加すると、モーダル外の入力に古い12pxの文字サイズが残り、隣の操作ボタン13pxと不一致だった。共通／会議別の用語ラベル・入力・補足を13px／行高20px、左右padding12pxへ揃えた。一行入力は40px、複数行の補足は60pxで縦リサイズを維持、checkboxは13pxのまま。項目内gap12px、ラベルと欄のgap8px、「有効」と削除ボタンの縦中心差0px、横溢れ0をCUA実測。保存処理・確認条件・業務データには変更なし。

追加source回帰1件は修正前27成功／1失敗、修正後28成功。最終フロント709件成功・失敗／skip0（runner1.590秒）、型検査＋本番ビルド成功（Vite1.50秒、全build経過未計測）、git diff --check成功。大きなJS chunk警告は残る。合成の無効入力は保存せずタブの再読込で片付けた。確認画像 `/private/tmp/mon-ui-spacing-vocabulary-final.png`。Mac QAへの今回追加の反映、狭幅の実測、全状態の検証は行っていない。Rust／CI／コミット／Pushなし。

## 2026-10-11 全体余白点検の追加：人数指定と録音準備

ユーザーの全体点検依頼に対し、既存の未コミットUI修正を保持して会議一覧、文字起こし本文／発言別設定／会議操作、人数指定、議事録本文／主操作、表示・保存設定、録音準備、通知を合成IPCのブラウザーで確認した。録音・AI処理・実データ保存は実行していない。

今回の追加はCSSと回帰テスト2件。人数指定の入れ子sectionには外側gridのgapが届かず、従来のmarginとラベル幅制限が残っていた。section自身へ16px gap、ラベルとフィールドへ8px gap、トラックごとの2列・全幅フィールドを設定。条件付き人数入力は同じトラック内に保持し、600px以下の1列ルールも追加した。見出し行のmarginも除き、infoとの中央を合わせる。空のジョブリストは余白を占めない。録音準備の一行入力と取得範囲selectは40px／13px／20pxへ統一し、取得範囲モーダルのラベルmargin重複も整理した。保存／同意／確認／disabled条件は変更していない。

CUA実測の1280×720で、人数指定のselect2欄と指定人数inputは全て高さ40px・文字13px・幅247px、selectの上端は一致。section gap16px、margin0、人数欄と開始ボタン間16px、見出しとinfoの中心差は2行とも0px、横溢れ0、内側scrollport0。Escで「人数指定・話者分離」入口へ復帰した。初回追加CSSは直下selectorのため二重ラッパー内sectionへ適用されず、実測で判明して実際の子孫構造へ修正した。source契約テストだけではこの構造を検出できないため、最終値をブラウザーで確認した。

録音準備inputも40px／13px／20px。取得範囲selectとラベルの追加ルールはsource契約であり、今回の録音ヘルパー未配置の合成画面ではその状態を表示できず実測証拠には数えない。通知の成功アイコン・本文・info・閉じるの縦中心は全て35px、card clientHeight=scrollHeight=40px、host overflow=visible。議事録の本文と主操作・発言別設定は既存配置を保持し追加変更なし。

viewport変更を試みたが既存／新規タブの実測innerWidthは1280pxのままで、960px／560px検証の成功には数えない。overrideは解除、余分な確認タブは閉じ、文字起こし確認タブだけ保持した。画像 `/private/tmp/mon-spacing-count-settings-final.png` は合成内容で私的な会議本文を含まない。

追加テストは修正前25成功／2失敗、修正後27成功。最終フロント708件成功・失敗／skip0（runner1.599秒）、型検査＋本番ビルド成功（Vite1.50秒、全build経過未計測）、git diff --check成功。JS chunk警告は残る。Mac QA／Rust再構築は行っておらず、保持バンドルに今回CSSが入ったとは扱わない。CI・コミット・Pushなし。全状態・実IME／VoiceOver・計画全体の完了を証明したものではない。

2026-10-10。ユーザー指定のUI集中改善であり、未コミット・未Push。基準HEADは `aa4f91a2757b16f9e00d48153e751f8f76c9b17b`。正式評価・10話者・Windows固有部分とWindows CIの保留は維持する。計画全体・M3-08／M4-01は進行中。

## 変更

細かい設定を明示操作のOptionsModalへ整理した。入力・保存queue・既存controllerは常時マウントし、閉じるだけで保存・実行・問題解除をしない。背景の状態変化は入口の「要確認」表示に留め、勝手にモーダルを開かない。閲覧用の目次・比較・ID表示と一覧の操作メニューは別扱い。既存の保存・再試行・対象照合・不明な結果の保護は維持し、新しい業務機能・IPC・権限・モデル取得は追加しない。

文字起こしは本文の直接編集と名前ラベルからの名前変更へ整理し、色付き背景はアバターだけとした。再生dockとトーストの独立スクロールを除去し、通知は1件と待機件数へ整理した。議事録の設定／詳細書式は主操作列へ、編集管理は設定内へ移した。

追加のユーザー指摘により、話者見出しと「すべての発言」の間へ16pxの余白を確保。文字起こしの編集オプションを「表示と再読込」「保存と回復」「まとめて話者を変更」に分け、見出し・区切り・フィールドの上下配置・ボタンの10px間隔を統一した。保存関連の3操作は同じ列へまとめ、選択解除は補助ボタンにした。会話の割り当てや保存条件そのものは変更しない。

## 再現した不具合

- DOM順とnative top-layer順が異なる三段の設定で、確認サービスが最初のdialogを親に選び、確認が背面へ隠れた。合成CUAで前面のAX操作が消え、背面の管理設定だけが確認へ置換されることを再現。showModal順の共通記録を確認サービス／説明ポップアップと共有し、最前面だけを親に選ぶ。不明なnative dialogは迂回しない。
- Mac WebViewではポインターで開いた設定ボタンに焦点が残らず、Esc後に会議ワークスペースへ戻り、次のTabが先頭のテーマ説明へ進んだ。OptionsModalと再生設定の入口を明示的にfocusしてから開く。比較QAではEsc後に「本文と再生の表示設定」へ戻ることを実機確認した。

## ブラウザーの実操作証拠

製品React＋合成IPCをCUAで操作。実会議の解析・録音・モデルは含まない。

- 三段の設定と最深部の実行前確認を表示し、Esc取消後のopener・入力保持・説明ポップアップの表示／Esc復帰を確認した。
- 無関係な確認の待機、処理中Esc拒否、完了後に次の確認だけが表示されることを確認した。
- 議事録の段落挿入後、詳細書式を閉じると挿入位置へ戻り、入力後の編集終了で保存済みと閲覧紙面への反映を確認した。
- 960×640の話者見出しとフィルターの間は16px。編集オプションは幅720px、clientHeight／scrollHeightとも577px、clientWidth／scrollWidthとも718px。全ボタンが画面内に収まり、モーダル内の独立scrollportは0件。
- 500×640では幅468px、clientWidth／scrollWidthとも451px、フィールドは1列。内容が長いため外側dialogだけがスクロールし、内側の独立scrollportは0件。これは製品の960px最小幅の合格とは別の応答的配置検査。
- 回復記録を開いてEscで戻り、さらに編集オプションを閉じて再度開いても割り当て先「佐藤」を保持。実際の一括割り当てや回復入力破棄は実行していない。

画面証拠: `/private/tmp/mon-speaker-heading-gap-final.png`、`/private/tmp/mon-transcript-options-layout-final.png`、`/private/tmp/mon-nested-confirmation-owner-final.png`。画像だけで保存／全異常系の成功を推測しない。

## 検査と時間

前回のフロント677件成功・失敗／skipなし。最後の単独buildは6.55秒、testは1.82秒。UI／確認サービス変更のため通常Rust／Swiftテスト全体は再実行していない。git diff --check成功。Viteの大きなchunkとSwift cacheの警告、公証skipは残る。

最初のQA構築184.49秒、確認所有先修正後の同じtargetで111.10秒。Mac固有の焦点修正を新しい専用targetで構築した比較QAは189.30秒。これらは途中QAで、最終余白修正の実機証拠には数えない。内部のフロント／Rust／Swift／署名は未分離で、単独buildと重複している。

最終レイアウトQAの構築は116.88秒で成功。4回のQA構築参考小計601.77秒（10分01.77秒）で、最終116.88秒以外は途中成果物。単独フロントbuild 6.40／7.74／6.55秒は合計20.69秒、test 1.67／2.05／2.28／1.82秒は合計7.82秒。内部工程と並行を含むため、合算を経過時間・性能値へ置き換えない。調査・実装・GUI・記録は個別未計測。

## 最終Mac QAの限定実機確認

最終QA `/private/tmp/mon-transcript-options-final-qa.OtGCAh/MON QA.app` を保持。`jp.mon.desktop.qa`・全3実行ファイルarm64・全コピー一致・両helper／notice／inventoryの生成元とのbyte一致・strict deepアドホック署名を確認した。本体SHAは `654597ce5367de59eba4b8c6d439f0ad548e3c63f8308f15cccb57e91c040ee5`。Developer ID署名・公証・Gatekeeperの証拠ではない。

Mac解除後、比較QAの終了・main不在を確認して最終QAを指定パスから冷起動。保存済み3会議を保持し、既存M4A会議で再生設定をモーダル表示、Esc後に「再生設定」へ焦点復帰を確認した。会議操作→エンジン設定も前面に開き、Escでエンジン設定の入口、さらに会議操作の入口へ一段ずつ戻った。パス・設定値・会議名は変更せず、解析／録音／削除は実行していない。最後にCmd-Qで終了し、main不在を読取りで確認した。

会議保存先は既存のQA専用 `/private/tmp/mon-native-ui-smoke.SOSoB9/MON-meetings-f352507d-7b63-4335-b953-6b063348c5fe`。終了後のreadonly DB検査は整合性ok・外部キー違反0・会議3・解析版0・管理モデル0。したがって生成済み文字起こし／議事録の余白・編集保存のMac実機確認は未達で、上の合成ブラウザー証拠と区別する。画面証拠 `/private/tmp/mon-native-playback-settings-modal-final.png` は最終QAの再生設定。表示設定の比較画像 `/private/tmp/mon-native-modal-settings-final.png` は焦点修正前のQAである。

終了後の追加冷起動を試みた時点ではMacが再ロックされ、CUAで操作できなかった。この追加試行は成功に数えない。先に確認した最終QAの冷起動・3会議保持と区別する。

## 追加のキーボード回帰検証

M3-08・仕様14.14の入力操作を確認。通常のCommonModalだけがIME／反復ガードの対象外になっていた。開発専用のボタンから確認欄にisComposing付きEsc／反復Escを送るとdialogが閉じることをCUAで再現した。共有のmodalKeyGuardへ整理し、通常React経由と外部コンテンツのnative経由の両方に適用した。変換中・keyCode229・処理済みイベントは入力側へ任せ、反復Escのみ既定動作と伝播を止める。最前面の対象照合・処理中guard・通常Esc取消・Tab循環は維持する。

修正後、isComposing／keyCode229／repeat付きのEscを合成ボタンから送り、dialog1件と確認欄「変換中の入力を保持」の値・焦点を保持したことを確認した。通常Escでは0件となり「確認を開く」へ復帰。処理中のEscでは1件と無効なキャンセルを維持し、合成完了後のEscでは待機していた「次の対象の確認」だけが表示された。三段設定の最深部から通常Escで二段へ戻り、入口焦点と再表示後の詳細入力を保持。最深部の先頭から逆Tabで末尾、末尾からTabで先頭へ循環した。

最新フロント680件成功・失敗／skipなし。最終build 5.76秒、test 1.62秒。途中検査はbuild 6.00秒・test 1.84秒で、参考小計はそれぞれ11.76秒・3.46秒（並行起動、工程時間の和を経過時間にしない）。検証専用UIの余白整理後も3種類の合成Escと入力保持を再確認した。実装・調査・CUA・記録は個別未計測。Viteの大きなchunk警告は残る。Rust／Swift／QA再構築なし、保持中のOtGCAhバンドルは前回677件時点で今回のキー修正を含まない。実IMEの変換候補取消や実キー長押しのMac動作は未検証で、合成KeyboardEventの証拠から合格扱いにしない。記録画像 `/private/tmp/mon-modal-key-guards-confirmed.png`。一時検証タブは閉じ、ユーザーの文字起こしタブを保持した。コミット・Push・CI・権限変更・解析／録音は行っていない。

## 追加の議事録確認とトースト中央揃え

議事録編集の回帰driverには、旧画面の展開パネル・編集領域内の主操作・背面本文の選択を前提とした経路が残っていた。編集は主操作列、編集管理は設定内のモーダル、書式詳細は本文で対象を選択してから開く経路に更新し、確認終了後は設定を閉じて編集位置へ復帰する。driverの構文／source契約のテストは成功したが、CLIの全行程は実行しておらず、破棄・保存失敗／明示再試行の全ケースの合格とは扱わない。合成画面だけに保存遅延の選択と保存文書の件数・生成原文一致のreadoutを追加し、製品のIPCや保存処理には追加しない。

CUAで書式モーダルからブロック削除を確認したところ、取消ボタンが設定由来の「閉じる」で、Esc取消後も削除入口ではなく設定の閉じるボタンに戻っていた。確認選択中は「キャンセル」、確認失敗時は「閉じる」、設定通常時は既存ラベルへ分けた。書式操作は本文の選択を壊さないpointer処理を維持し、削除確認へ押されたボタンを明示的なopenerとして渡す。

修正後の合成CUAでは安全な初期焦点「確認をキャンセル」、対象ブロック3と損失範囲、取消後の削除入口復帰を確認。取消時は保存要求0・保存済みブロック6／項目2／根拠2・生成原文保持。承認後の3秒遅延中にdialogへEscを入力しても確認を維持し、キャンセルと実行ボタンを無効化した。遅延応答後は保存要求が1回増え、ブロック5／項目1／根拠1となり、書式を閉じると残ったブロック3へ焦点復帰。別の往復で編集管理からUndoを1回実行し、ブロック6／項目2／根拠2と本文・根拠チップを再表示した。件数と表示の検査であり、実DBへの保存／全内容のbyte一致の証拠ではない。生成原文のJSON一致readoutは終始保持。最初の遅延検査は無効ボタンへpressを試みて待機し、応答後の状態しか確認できなかったため、処理中Esc拒否の証拠には数えない。次の実行でdialogへ入力して処理中表示を確認した。記録画像 `/private/tmp/mon-minutes-nested-cancel-label.png`。

ユーザーのトースト上下ずれの指摘に合わせ、共通toast-cardをflex中央揃えにし、種別アイコンと閉じるボタンもinline-flex中央揃え・line-height1へ統一した。期限・重要度・通知queue・操作・情報ポップアップは変更しない。合成CUAで「表示設定を保存しました。」の1行カードは高さ58px、全4子要素の中心Y=113。2行カードは高さ67.59375pxで全中心Y=117.796875。操作付きカードは本文＋操作の領域を中央へ配置し、全中心Y=131.3984375。ヘッダー内の操作付きカードは高さ44.3984375px、各中心の差は0.004px以内で、hostのoverflowはvisible。全通知文・テーマ・実WebViewの合格とは区別する。画面証拠 `/private/tmp/mon-toast-center-single.png`、`/private/tmp/mon-toast-center-multiline.png`、通常表示の抜粋 `/private/tmp/mon-toast-center-final.png`。

最新684件成功・失敗／skipなし。最終build 6.01秒・test 1.90秒、議事録修正後の途中検査683件はbuild 5.83秒・test 1.73秒。参考小計はbuild11.84秒・test3.63秒で、それぞれ並行起動を含むため経過時間へ置き換えない。調査・実装・GUI・記録は個別未計測。Viteの大きなchunk警告は残る。git diff --check成功、コミット・Push・CIなし。検証用タブだけを閉じ、ユーザーの文字起こしタブを保持。Rust／Swift／QAは再構築せず、保持中OtGCAhバンドルに今回の修正は含まれない。計画全体の未達と正式評価／Windows保留は変わらない。

## 保存失敗・破棄確認の追加検証

M3-08と仕様14.13／14.14／14.18の保存・回復・入力操作を継続。合成CUAで削除承認後の保存失敗を再現すると、CommonModalの実行／取消ボタンの無効化で焦点がBODYへ落ちた。確認の識別子を照合し、実際の最前面だけで、BODY／dialog／確認見出し／そのdialog内の無効要素に残った焦点だけを回復する。処理中は確認見出し、失敗後は有効な閉じるボタンへ移す。他の有効な操作への移動や、背面の確認からの焦点奪取はしない。入れ子の所有確認とConfirmationHostの独立確認に同じ補正を適用した。

修正後の入れ子確認では、失敗後のactiveElementが「確認を閉じる」でdialog内にあることを確認。Enterで閉じ、書式設定を閉じてもローカルの削除内容・保存要確認・再試行が残った。保存要求1、保存済みブロック6／項目2／根拠2・生成原文保持に対して、紙面はローカルの5ブロックを表示。合成保存先を復帰し通知を閉じても保存要求1を維持し、主操作の明示再試行だけで2へ増え、保存済み5／1／1に反映した。全内容のbyte一致や実DBへの保存の証拠ではない。

別の初期状態からブロック2を変更し、設定→編集管理→再読込へ進むと、復元できない未保存入力の破棄確認となった。確認中の保存要求は0で、Esc取消後も変更文字列を保持し、再読込の入口へ復帰。取消後に自動保存が再開して合成保存失敗1件となったことと、確認中の保存停止を区別する。再度の破棄を明示承認した後だけ元の「決定事項」へ戻り、保存済み6／2／2と生成原文を保持。破棄承認で不要な保存要求は増えなかった。

独立確認の開発用HTML入口と成功／3秒遅延／応答喪失の選択を追加した。製品bundleの機能・IPC・権限ではない。独立確認でも処理中の見出し焦点とEsc拒否、応答喪失後の「確認を閉じる」焦点、Enter閉鎖後の呼び出し元復帰をCUAで確認した。記録画像 `/private/tmp/mon-minutes-save-failure-focus-fixed.png`、`/private/tmp/mon-root-confirm-failure-focus-fixed.png`。修正前の `/private/tmp/mon-minutes-save-failure-confirm.png` は焦点の不具合を含むため合格証拠にはしない。

最新685件成功・失敗／skipなし。最終build5.90秒・test1.84秒、途中検査build5.74秒・test1.60秒。参考小計build11.64秒・test3.44秒は並行起動を含み経過時間とはしない。実装／調査／GUI／記録は個別未計測。git diff --check成功、Viteの大きなchunk警告は残る。実モデル・実録音・権限・QA反映・Rust／Swift再実行・CI・コミット／Pushなし。Macのアプリ一覧取得は成功したが、それを最新QAの起動・実WebView検証の成功とは扱わない。合成検証タブだけを閉じユーザーの文字起こしタブを保持した。正式評価・Windows保留と全体の未達条件は変わらない。

## 最新685件時点のMac QA反映と実機確認

専用target `/private/tmp/mon-ui-modal-final-native.3W8TkM` で最新UIのQAを構築、188.09秒で成功。内部フロント／Rust／Swift／署名は未分離で、通常Rust／Swiftテスト全件の合格ではない。並行したフロント再検査は685件成功・失敗／skipなし、2.13秒（test runner本体1.9035秒）。実装／GUI／調査／記録は個別未計測。この節は前節までの「保持QA未反映」を更新する。

保持QA `/private/tmp/mon-ui-modal-final-qa.bbITMx/MON QA.app` のコピー全体一致・strict deepアドホック署名・識別子 `jp.mon.desktop.qa`・本体と両helperのarm64・両helper／notice／inventoryの生成元byte一致を確認。本体SHA256は `280bca52cc44818176077222f629376d45d6d587df2a40328aa5e06425f97300`。Developer ID署名・公証・配布の証拠ではない。環境はmacOS27.0（26A428）。

CUAでこの正確なパスから冷起動し、読み取りプロセス照合で同じ本体を確認。既存3会議とM4A会議の未解析状態を保持し、会議操作→エンジン設定でEscを1回ずつ入力すると、エンジン設定の入口、会議操作の入口へ順に戻る。再生設定もEscで入口へ戻った。設定画面でQAの「再生位置に追従」を0→1へ変更した保存成功トーストを実WebViewで撮影し、本文・種別アイコン・info・閉じる操作の中央配置を確認。その後0へ戻して保存通知を確認し、テーマ・パス・モデル設定は変更していない。通知詳細へのクリック時は要素が失効したため、その詳細操作の合格とはしない。

記録画像: `/private/tmp/mon-native-latest-toast-center.png`、`/private/tmp/mon-native-latest-playback-modal.png`。座標の数値一致は前節の合成DOM測定、今回の実機は表示確認で区別する。実IME／長押し、生成済み本文の実機編集保存、保存失敗の実DB異常系、全通知文／テーマは未検証。解析版・管理モデルが0件であるため、モデル統合の合格にも数えない。

Cmd-Q後の読み取り確認でQA mainと専用ビルドプロセスは不在。QA専用DBはintegrity_check=ok、foreign_key_check=0行、会議3・解析版0・管理モデル0。起動直後のアプリ一覧に残っていたrunning表示だけを終了証拠にしない。終了済み専用targetはリンクでないこと、inode170873122・uid501・mode40700を照合して回収した。1,242,780KiB（約1.19GiB）の再生成可能な構築物のみ削除、最新QA37,608KiB・共有Rust8,415,912KiB・製品target38,668KiB・ソース・ログ・画像・既存データは保持。録音／モデル実行・TCC変更・CI・コミット／Pushは行っていない。

## 保存済み編集文書の完全一致と取消後の自動保存

開発用の文書整合性readoutへ、初期編集文書とのJSON一致を追加した。ブロック・項目・根拠の全内容を比較し、生成原文の一致とは独立させた。本文と根拠が同じ件数のまま変更された場合に不一致となる単体検査も追加。実文書を画面へ出力するのでなく一致結果だけを表示する。製品コード・IPC・保存条件は変更していない。

CUAの通常UI操作で項目付きブロック3の削除を明示承認し、保存要求1・保存済み文書5ブロック／1項目／1根拠・初期JSON不一致・生成原文保持を確認。設定→編集管理から直前の編集の取消を1回実行すると、保存済み文書6／2／2・Undo要求1・初期編集文書のJSON一致・生成原文保持へ戻った。単なる件数一致より強い証拠だが、合成IPCでの文書比較であり、実DB・実AI結果の証拠ではない。記録画像 `/private/tmp/mon-minutes-undo-json-exact.png`。

破棄検査driverは、取消後に再開する自動保存より先に再読込を押すタイミングを暗黙に必要としていた。合成保存失敗を先に設定し、取消後の失敗を待ってから再度破棄を確認する経路へ修正。新しい合成画面から本文を変更し、破棄確認中は保存要求0であることを時間を隔てて確認した。Esc取消で入力と再読込入口焦点を保持し、通常の自動保存が合成失敗1要求となっても本文を保持。明示した破棄承認後だけ保存済みの「決定事項」へ戻り、要求1を維持し、初期JSON一致・生成原文保持を確認した。取消後にも要求0を期待して自動保存を抑制する変更はしない。

最終フロント686件成功、失敗／skipなし。test1.69秒・build5.82秒（並行起動）。途中の全686件／重点7件も成功したが個別のwall時間は未計測、最終値へ加算していない。最初のnpm引数による名前フィルターは適用されず全件実行となった。実装／調査／GUI／記録は個別未計測。Vite大chunk警告は残る。driverの構文／契約と上記CUA経路は確認済みだが、CLI全行程の実行合格ではない。変更は検証専用ファイルと記録だけで、最新QAの製品ソースに追加変更なし。Rust／Swift／QA再構築・CI・コミット／Push・録音／実モデル／権限変更なし。一時検証タブだけを閉じ、ユーザーの文字起こしタブを保持した。計画全体の残件は変わらない。

## 隣接するボタンの寸法・フォント統一

ユーザーの追加指摘に対し、合成CUAで議事録の「編集を終了／書式の詳細／Markdownを書き出す／設定」を実測。高さ40／34.3984375／40／34.3984375px、文字は12pxでも行高20.4px、太さ500／500／650／500で不揃いだった。CSSの共通action寸法へ整理し、通常40px・文字13px・行高20px・太さ500、コンパクトな操作列32px・文字12px・行高18px・太さ500に統一。設定入口のポータルにも継承し、モーダルとその内側の操作列は通常寸法へリセットする。長文は最小高で折返しを許し、同じflex行はstretchする。名前と発言件数も文字13px／太さ500で揃え、行内の高さをstretchした。アイコン・本文直接編集・根拠チップは文字ボタンと区別し、ナビゲーション／タブの文字15pxは保持。

修正後の合成CUAでは、議事録の4操作はすべて高さ32px・文字12px・行高18px・太さ500・同一font-family・上下padding6px。書式モーダルの閉じると16操作は高さ40px・文字13px・行高20px・太さ500。文字起こしの確定／キャンセルと、共有再生／再生設定も各32px・12px・18px・500。設定画面の可視12文字ボタンは40px・13px・20px・500。通常／無効／色の異なるボタンで同じ寸法を確認した。全OS／全画面／全フォントの証拠ではない。

960×640で文字起こしの編集中編集オプション7ボタンは40px／13px／20px／500。dialog高さ581.671875px、clientHeight=scrollHeight=580、clientWidth=scrollWidth=718で横溢れなし。未編集状態のdialogは547.578125px、内側scrollport0。一度「編集を開始」を探した時は既に直接編集から編集状態へ入っていたため該当なしとなり、再取得した実画面の全ボタンで検証した。viewportは最後にreset。記録画像 `/private/tmp/mon-adjacent-minutes-buttons-clean.png`、`/private/tmp/mon-adjacent-transcript-options-editing.png`。途中の復元確認前の画像は最終レイアウト証拠に置き換えない。

最新687件成功・失敗／skipなし、最終test1.76秒・build6.28秒。途中検査test1.82秒・build6.57秒も成功し、参考小計はtest3.58秒・build12.85秒。並行起動を含み、これを実経過時間とはしない。実装／調査／GUI／記録は個別未計測。Vite大chunk警告は残る。git diff --check成功。CSSが変わったため、保持QA bbITMxはこの寸法変更未反映。今回Rust／Swift／QA再構築・CI・コミット／Pushなし。

## 同時に確認したMarkdown出力と直前版復元

開発用の合成操作に出力結果・応答喪失・採用／復元応答と件数readoutを追加。製品bundleの業務機能・IPC・権限ではなく、実保存ダイアログや実ファイルの書き出しは行わない。新設した生成完了・別対象応答の操作は今回未検証。

通常UIからMarkdown出力を順に操作し、成功は要求1・成功履歴1と成功通知、取消は要求2・履歴追加なしと情報通知、失敗は要求3・失敗履歴追加と要対応を確認した。失敗通知を閉じても要対応と設定内の失敗履歴を保持し、新しい出力要求は増えない。次の合成成功で応答喪失を起こすと要求4・成功履歴追加・「前回の書き出し結果を確認」となり、結果確認で要求4を維持して結果読込だけ1へ増えた。現在版a／直前版pと初期編集文書のJSON一致・生成原文保持は維持。これは通知・保存結果照合経路の検査で、Markdownバイト内容／実容量不足／OSダイアログの証拠ではない。通常成功の閉じる操作を探した時には既に通知が期限終了しており、その閉じる操作を合格扱いにしない。

設定→直前版比較→復元確認でEsc取消し、復元要求0と「この議事録直前版へ戻す」への焦点復帰、設定＋比較の2モーダルを確認した。明示復元で合成応答を喪失させると現在版p／直前版a、復元要求1、失敗確認の閉じる焦点を保持。失敗確認・比較・設定を一段ずつ閉じ、結果だけを読み取り確認すると画面の現在版#1／「前回の決定」と検証済み成功通知へ更新し、旧編集文書aの初期JSON一致・生成原文保持を維持した。最初の読み取り後の即時観測では未確認状態が残っていたため、その時点を成功とは数えず、後の状態再取得と読み取り確認で成功表示を確認した。情報ポップアップdivへのpressは失敗し、説明の閉じるボタンへ切り替えた。復元を再送しておらず、結果確認は旧版と新しい直前版を反転させない。

旧CLI driverの全行程は未実行で、設定モーダルの親を含むdialog件数や待機条件の更新も残す。合成タブだけを閉じ、ユーザーの文字起こしタブを保持。正式評価・Windows保留と全体の未達条件は変わらない。

## 採用後に消える入口と親モーダルへの復帰

現行の設定→候補比較→採用確認で明示採用した後、比較入口が消え、開いたままの設定モーダルの外にある「議事録」見出しをfallbackに選ぶためBODYへ焦点が落ちる不具合を合成CUAで再現した。代替先は親モーダルがある場合その内側の見出し（なければ有効な取消操作）とし、残る最前面モーダル外・inert・閉じた要素を復帰対象から除外した。fallbackの通常ボタンはTab対象から外さず、実際に焦点が移った場合だけ復帰済みとする。別のモーダルが最前面の場合の復帰保留は保持した。

修正後の新しい合成画面で、比較Esc取消は「議事録候補を確認」に復帰。再度明示採用すると焦点は設定モーダル内H2「議事録の設定・生成状況・候補」、開いているnative dialogは親設定1つ。そこからTabで閉じる操作、Escで主操作列の設定入口へ復帰した。現在版b／直前版a、採用要求1・復元要求0、旧編集文書6ブロック／2項目／2根拠の初期JSON一致と生成原文保持を確認。実DB・実AI結果・実機候補採用の証拠ではない。記録画像 `/private/tmp/mon-candidate-parent-focus-fixed.png` はその後再度設定を開いた画面であり、採用直後の焦点はDOMの数値・文字観測で記録した。readoutのwaitForにstateを指定し忘れた1回は観測エラーとなり、後のAX／innerTextで件数と保持を確認した。

候補／復元driverは設定を明示して開き、比較を閉じた後も親設定が残ることと、確認は親設定＋比較所有の2native dialogであることを検査するよう更新。出力通知driverは主操作列から出力し、履歴を見る時だけ設定モーダルを明示して開く。構文検査成功、CLI全行程の実行とは区別する。

フロント最終688件成功・失敗／skipなし、test1.60秒・build5.77秒（並行起動）。Vite大chunk警告は残る。実装／調査／GUI／記録の個別wall時間は未計測。git diff --check成功。録音／実モデル／TCC変更・CI・コミット／Pushなし。

## ボタン統一と親モーダル復帰の最新QA構築

専用target `/private/tmp/mon-button-modal-native.Xrv5Si` でまとめ構築181.45秒。保持QA `/private/tmp/mon-button-modal-qa.X1luwR/MON QA.app` は生成元との差分なし、identifier `jp.mon.desktop.qa`、strict deepアドホック署名成功。main・capture・embeddingの全3実行ファイルarm64で、capture bundle・embedding・notice・inventoryはソース側構築物と一致。main SHA-256 `be755dfaf4de14373a956b7156c345b322675bd3d49bd90ea11e68e09f587d30`。captureの実行名を最初にMONCaptureProbeと取り違えたfile検査は未成立で、Info.plistと一覧から実名mon-macos-captureを確認して再検査した。公証は未設定でスキップ、配布署名の証拠ではない。第三者licenseReviewも未検証のまま。

Macが再ロックされ、CUAのアプリ一覧取得時に解除を求められたため、最新QAの起動／実WebView確認は未実施。解除をユーザーへ依頼済み。この構築成功を最新UIの実機合格に置き換えない。元の合成一時タブは閉じ、ユーザーの文字起こしタブは保持した。

構築プロセス終了後にtargetを使うプロセス不在、リンクでないこと、inode170892809／uid501／mode40700を照合して専用target1,242,024KiB（約1.18GiB）を回収。再生成可能な構築物だけを削除し、QA37,608KiB（約36.73MiB）・製品target38,668KiB・共有Rustキャッシュ・ログ／画像・既存データは保持。回収後も保持QAのstrict署名成功。Rust／Swift全テストの再実行ではなく、構築に含まれるhelperと製品release buildのみ。

## 最新QAの実WebView操作と採用／復元の回帰検証

次の継続時にはMacが解除されていた。保持QA X1luwRをCUAから起動し、読み取り専用のプロセス照合でその正確なmainパスを確認。既存3会議を保持した実WebViewで議事録の未生成状態の「議事録を生成／設定」と共通音声の「会議全体を再生／再生設定」の高さ・文字配置が揃うことを画像確認した。32px／font数値の実機DOM計測ではなく、前節の合成数値と区別する。生成済み本文がないため編集操作列の実機証拠にはしない。

再生設定は安全な閉じる初期フォーカス、Escで入口復帰。議事録設定は閉じる初期フォーカス、Tabで再読込、逆Tabで閉じる、Escで主操作列の設定入口へ復帰。会議操作→文字起こしエンジン設定の入れ子も閉じる初期フォーカスで、Esc1回でエンジン設定入口、もう1回で会議操作入口へ戻った。値の入力・保存・録音・解析・モデル取得／実行・TCC変更はしていない。画像 `/private/tmp/mon-native-button-alignment-minutes.png`、`/private/tmp/mon-native-button-alignment-playback.png`、`/private/tmp/mon-native-button-alignment-settings.png`。生成済み本文の実機編集保存・採用／復元、実IME／VoiceOver、全異常系は残る。

Cmd-Q後のmain不在を読み取り確認。QA専用DB integrity_check=ok、foreign_key_check=0行、meetings3／analysis_revisions0／model_packages0。最初の件数照合は誤ったanalysis_versions表名で途中エラーとなったため、sqlite_schemaから実名を確認し全件を再検査した。初回の部分成功を完全検査としない。

publication通知driverは主操作列の議事録タブ・設定モーダルを明示し、比較の失敗確認→比較→親設定を一段ずつ閉じてから読み取り結果確認へ進むよう更新した。設定保存／生成開始のエラー通知を閉じる時も、背景がinertのままクリックせず設定を先に閉じる。設定の入力保持・明示再試行の検査時は再度設定を開く。新規検証タブのattachが2回timeoutし、一覧上もそのタブがないことを確認したため、異常系の追加CUA全行程は未成立。既存ユーザーの文字起こしタブは変更せず保持。

synthetic fixtureと製品のreceipt／head照合を組み合わせ、採用／直前版復元それぞれに応答喪失・別対象応答の4組を追加検査した。応答喪失はthrow、別対象応答は製品照合で拒否し、同じ確認対象をget_minutes_versionsで読み取り照合すると要求した版・旧現在版・lock増分が一致する。各変更命令は1回だけ、旧編集文書のレコード全体と生成原文のJSON一致を保持。React controllerの画面操作や実DBの証拠ではなく、pending／通知の全ルートをこの単体検査だけで完了扱いにしない。driverの構文・現行モーダル経路の契約も追加検査したがCLI全行程は未実行。

最終フロント690件成功・失敗／skipなし。test1.72秒・build5.99秒、並行実行で小計を実経過時間としない。実装／調査／GUI／記録の個別wall時間は未計測。Vite大chunk警告は残る。製品ソース変更なし、保持QAは今回の製品ソースに一致するため再構築／Rust／Swift全テストは行わない。CI・コミット／Pushなし。計画全体の達成は未証明で、正式評価・Windows保留とその他の未達条件を保持する。

## 実モデル統合の配置前提の再確認

実AIの統合へ進む前に、リポジトリ全体（hidden／ignore込み）・`.cache`・`/private/tmp` のファイル名を読み取り確認した。候補拡張子GGUF／ggml bin／segmentation・embedding ONNXと、whisper-cli／llama-completion／sherpa diarizationの実行名を検索した範囲では該当実ファイルがない。以前の検証記録にある `/private/tmp/mon-whisper-integration.ibH0yg` と `/private/tmp/mon-llama-integration.dq3nxo` はdirectoryとして残るが、以前の絶対パス4点の実ファイルチェックは全て不在。directoryが残ることをモデル利用可能の証拠にしない。確認範囲外・別名の端末内モデル不存在を証明したものでもない。

QA app-dataのmodels領域は未作成、既存QA DBのmodel_packagesは0件。設定キーを再確認しようとした追加SELECTは誤ったsettings表名でエラーになったため、設定の新しい読取り証拠には使わない。実機画面で前節の空パスを確認した証拠と区別する。提供済み先行評価FLACは452,246,304bytesで存在し、今回本文や音声を読まず、Gitへ追加していない。

新規ダウンロード／実モデル実行・録音・設定変更は行わず、再取得または既存配置先の指定が必要と判断した。実AI統合・配布条件・録音／TCC・長時間・署名／公証・先行試用の未達条件は維持。製品ソース変更なし、以前の690件／build成功を今回の再実行とはしない。実装／調査／記録時間は個別未計測、CI・コミット／Pushなし。

## 再取得許可後の生成済み本文の実機確認

ユーザーの公式モデル再取得／QA内実行許可後、同じX1luwR QAの通常画面から実whisper／sherpa／llamaの30秒統合を進めた。文字起こしと議事録の候補を明示確認・採用し、両本文のQA編集保存とCmd-Q／再起動後の保持、保存編集を含む実Markdown出力まで確認。合成fixtureの成功だけだった生成済み本文の基本保存経路には実機証拠が加わった。話者分離は候補保存だけで現在版を自動変更しなかった。

一方、一部設定／候補モーダルでnative AXが空になり座標操作を要したため、実VoiceOver／IME、全モーダルのアクセシビリティ・全異常系・UI全面完了とはしない。小型モデルの精度や要約の重複も正式評価の合格ではない。詳細と失敗を含む時間・容量・hashは [実AI再取得の記録](real-ai-reacquisition-2026-10-10.md) を参照。QA終了・元音声／本番設定保持、CI・コミット／Pushなし。下記残件のうち生成済み本文の基本保存はこの節で更新したが、全行程は残る。

## 保持・残件

最初の終了済み専用target `/private/tmp/mon-modal-ui-qa-build.xkIIE4`（約1.18GiB）と中間QA `/private/tmp/mon-modal-ui-qa-artifact.qwl26m`（約36.73MiB）は、関連プロセス不在・コピー一致・署名・device／inode／所有者／modeを確認して回収した。共有Rust約8.0GiB・製品target約38MiB・既存会議／音声／モデルは保持する。

最終実機確認後、終了済み専用target `/private/tmp/mon-native-modal-focus-build.WBOYZw`（約1.18GiB）と、比較QA `mon-modal-owner-final-qa-artifact.7Gm4KT`／`mon-native-modal-focus-artifact.bRExwV`（各約36.73MiB）も、関連プロセス不在・生成元コピー一致・device／inode／所有者／mode・リンクでないことを確認して回収した。今回合計2,598,004KiB（約2.48GiB）の再生成可能な構築物。最終QA約36.73MiBを保持し、回収後もstrict署名成功。ソース・ログ・画像・過去の既存QAとユーザーデータは削除していない。

全通知・全異常系・実IME／VoiceOver・生成済み本文のMac編集保存・実モデル統合、録音／TCC／4時間、Developer ID／公証／更新、先行試用は残件。正式評価・10話者・Windows固有部分とWindows CIは保留。コミット・Pushはユーザー指定で引き続き行わない。

## 2026-10-11 余白・隣接要素の追加点検

対象は会議一覧、文字起こしの本文・未解析表示・編集オプション、解析候補比較、議事録の表示・編集、表示設定、録音準備、再生設定、通知。既存の未コミット変更を保持し、コミット／Push／CI／Rust構築は行わない。

修正した箇所:

- 文字起こしの未解析・読込中表示に専用の余白を追加。詳細画面は上下24px・左右32px、760px以下は上下20px・左右16px。エラー／保存通知の左右余白も狭幅時16px。
- 候補比較の時刻・話者・再生操作を折り返せる中央揃えの行へ変更。文言ボタンは32×32pxの再生アイコンに置き換え、時刻入りの読み上げ名、ツールチップ、locked／準備中の禁止条件、既存の音声要求と下書き照合を保持。
- 共通の文字ボタン規則から再生アイコンを除外。幅が6pxまで圧縮されていたSVGを18×18pxへ修正。時刻との間隔は12px、縦中央差は約0.004px。
- 文字起こし・議事録の候補比較の重複した閉じるボタンを除去。CommonModalの単一の閉じる操作へ集約し、復帰先／処理中ガードを維持。比較モーダルは最大980px、760px以下は一列。
- 議事録の見出し・状態・操作列に左右32px（狭幅16px）の余白を確保。文字起こしの見出し・案内文の行は最小32pxにして編集オプションの中央位置と一致させた。
- 会議検索／名前変更入力の寸法、設定の選択欄、操作列の状態文、エラーと再試行の間隔、用語の有効チェックの間隔を統一。長い操作列は折り返し可能なまま維持。
- 再生設定でチェックボックスがラベル上へ配置されていた問題を修正。横並び・8px間隔・縦中央に戻した。取得同意／中断録音確認は長文に備えて先頭位置と8px間隔を確保し、削除同意の既存先頭揃えは変更しない。

検証はCUAによる合成データの実ブラウザ操作。1280×720、960×640、560×640で対象を確認した。編集オプションのボタン／選択欄は40px・文字13px、議事録の主操作／書式ボタンは32px・文字12px。560pxの候補比較は一列でモーダルclientWidthとscrollWidthはともに511px、発言メタ行も横あふれなし。未解析表示も560pxで左右16px、bodyの横あふれなし。文字起こしの見出し・案内・編集ボタンの縦中央は229.684／229.684／229.688px。

トーストの本文・成功アイコン・info・閉じるの縦中央はすべて113px、内側余白12px 16px、縦横スクロール差0。再生設定の追従チェックはdisplay:flex・align-items:centerで確認。合成fixtureは保存先APIを持たないため設定画面の保存先読込エラーが表示されるが、実製品の失敗の証拠ではない。画面幅変更は選択タブにだけ反映されるため、未反映の1280px読出しを狭幅の証拠には使わず、専用の選択タブでinnerWidth=560を確認して再検証した。終了時に幅設定を解除し、確認タブを閉じた。

画像は `/private/tmp/mon-ui-spacing-preview-2026-10-11.png`、`/private/tmp/mon-ui-spacing-minutes-2026-10-11.png`、`/private/tmp/mon-ui-spacing-transcript-2026-10-11.png`。フロントエンド自動テスト694件成功、型検査と本番ビルド成功。既存の大きいJSチャンク警告は残る。今回の最新変更を組み込んだMac QAバンドルの再構築／実WebView検証は未実施で、上記をネイティブ実機合格とは扱わない。過去の実AI／QA記録と区別する。

## 2026-10-11 最新余白調整のMac実機確認

上記の未反映状態を更新する追加検証。専用targetで初回QA構築184.66秒、Macの実WebViewでOS標準selectだけ余白／最小高の指定が反映されず、隣接ボタンより低く見える差を確認した。対象を表示設定・共通モーダル・音声操作のselectに限定してappearance:noneとテーマ色の矢印を指定。native selectの意味は保ち、forced-colorsでは標準外観へ戻す。回帰assertを追加した最終フロント694件成功（失敗／skipなし、1.498秒）。修正後のQA再構築117.46秒。2構築の実経過小計は302.12秒で、内部のフロント／Rust／Swift各工程の個別時間ではない。実装・GUI確認・記録の個別時間は未計測。

最終保持先 `/private/tmp/mon-spacing-select-qa.lFJAwI/MON QA.app` は生成元とdiffなし、strict deepアドホック署名成功。main SHA-256 `0e3f47a30688de1ddf2ae27d9bf25f074ac04be7192486cafded861b99b00a85`。この正確なパスをCUAから起動し、以下を通常UIで確認した。

- 未解析表示の左余白と、共通再生／再生設定の隣接配置。
- 再生設定の選択欄の高さ・左右余白、追従チェックの横並び。閉じる初期フォーカスからTab・Spaceでnative選択メニューが開き、Escでメニューを閉じ、次のEscで再生設定の入口へ戻る。設定値は変更していない。
- 文字起こし編集オプションの3区分、話者選択と再読込の同じ高さ、回復操作の間隔、一括割当先と適用／解除の整列。Escで編集オプション入口へ復帰。本文や話者の変更なし。
- 保存済み生成結果を含む議事録の通常／編集表示で、見出しの左右余白、保存状態の縦中央、主操作4ボタンと基本書式4ボタンの整列。編集へ入り、入力せず編集を終了。既存の保存確認マーカーを保持した。

最終実機画像は `/private/tmp/mon-native-spacing-select-fixed-2026-10-11.png`、`/private/tmp/mon-native-spacing-edit-options-fixed-2026-10-11.png`、`/private/tmp/mon-native-spacing-minutes-fixed-2026-10-11.png`。実機は画像による表示確認であり、合成ブラウザの40／32pxの数値計測と区別する。実録音の背景本文を含む画像は会話への添付には使わない。初回QAでは候補比較の再生アイコン・時刻間隔と、比較から親設定／元入口へのEsc復帰も確認したが、その初回バンドルには最終select修正が含まれていない。

Cmd-Q後に読み取りプロセス照合でQAと専用構築の不在を確認。初回のsandbox内psは拒否されたため、その出力を終了証拠にせず、許可された読み取りチェックでやり直した。readonly DB integrity_check=ok、foreign_key_check=0行、meetings4／analysis_revisions4／working_drafts2で起動前件数を維持。bootstrap SHAは前後とも `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020`。件数照合を本文全体のbitwise一致とはしない。

使用終了・コピー一致・署名確認後、専用target `/private/tmp/mon-spacing-native-build.oKPjgb` のリンクでないこと、inode170994469／uid501／mode40700を照合し、1,242,384KiB（約1.18GiB）の再生成可能な構築物だけを回収した。最新QA37,608KiB・共有Rust8,420,760KiB・旧QA・モデル・元音声・既存データ・ログ／画像は保持。Developer ID Applicationの署名identityは読取りで0件、公証も設定なしでスキップされたため、配布条件は未達のまま。録音・TCC変更・新規モデル取得／実行・CI・コミット／Pushなし。実IME／VoiceOver・全通知／異常系・長時間録音など計画全体の完了証明にはしない。

## 2026-10-11 全体余白点検の再追加

ユーザーの「他全体的に余白がない部分や、隣接する要素で上下の位置がずれている部分」の依頼に対応。既存未コミット変更を保持し、今回の製品変更は表示CSSだけ。新しい業務機能・IPC・保存処理は追加しない。

- 共通用語フォームは入力欄と次のラベルが密着していた。各項目の間12px、ラベルと入力の間8pxへ整理。有効チェックと削除ボタンも中央揃え。
- 設定の「中断・削除の管理」は直前カード境界と接していた。カードから上下18pxの余白を確保。共通用語などのカード見出しは17pxへ統一。
- 議事録／固定文字起こしなどのページ移動の入力欄と隣接ボタンを共通の高さ・字級へ統一。フォーム、版一覧、根拠確認は折り返しと間隔を保持。カード見出しにも12px間隔を確保。
- 説明ポップアップ見出しと閉じる操作、発言メタ情報の話者、チェックラベルの中央位置を整理。モデル定義ファイル選択のボタンとファイル名の間にも12pxを確保。
- 発言本文／議事録本文の右に8pxの余白とスクロールバー用の安定領域を確保。短い検索行と追従状態の独立スクロールは除去。長い本文やモーダル外側の必要なスクロールは保持。
- 設定モーダルの直下にある段落・操作列・入力ラベルの余分な上下marginを除き、親の16px gapへ統一。本文・入力値・フォーカス復帰条件は変更しない。

CUA合成UIで会議一覧、文字起こし、編集オプション、設定、モデル取込入口、用語フォーム、長い議事録、ページ移動、議事録設定、通知を点検。1280×720、960×640、編集オプションのみ500×640の追加応答的配置を確認。500pxは製品最小960pxの合格とは別。

実測は用語項目間12px／12px、管理入口と直前カードの間18px、有効チェックと削除ボタンの縦中央差0.004px。編集設定のボタンとselectは40px・13pxで各行の上端が一致、本文側／再生dockの主操作は32px・12px。議事録ページ移動の入力とボタンは40px・13px、上端が同じ行で一致、clientWidth=scrollWidth=694px。960pxの話者見出しとフィルターの間16px。500pxの編集設定は一列、clientWidth=scrollWidth=451px、内側の独立scrollportは0、長さによる外側dialogのscrollは保持。議事録設定もclientWidth=scrollWidth=543px・内側scrollport0。トーストの成功アイコン・本文・info・閉じるは中心差0px、clientHeight=scrollHeight=56px。設定保存先の合成API未実装エラーは実製品エラーとは区別する。

最終自動テスト699件、失敗／skipなし（runner1.482秒）。型検査＋本番ビルド成功（Vite工程1.52秒、型検査込みの個別経過は未計測）。今回UIだけの変更なので通常Rust全体の再検査は追加していない。git diff --check成功。JS chunk警告・配布licenseReview未確認・公証skipは残る。

Mac QA構築119.47秒はフロント／Rust／Swift／署名を含む経過で内部未分離。既存の一時targetを再利用し、別の保持先 `/private/tmp/mon-global-spacing-qa.ef5r1M/MON QA.app` へコピー。生成元とのdiffなし・strict deepアドホック署名成功・識別子jp.mon.desktop.qa・main SHA-256 `a7aa3594d5a7ab57ee115d54fc1bcb0c1d00e3ffe93c62be3ca3b21e502ae0be`。この指定パスから冷起動して既存8会議を表示し、通常UIで表示設定のselect・チェック・状態文と、保存済み文字起こし編集設定の3区分／隣接ボタン／selectを目視確認。双方Esc後の入口へ復帰し、入力・話者・設定・保存先を変更していない。Mac数値計測ではなく画像確認であり、合成の数値と区別する。

Cmd-Q後にQA本体と一時target関連の実行プロセス不在を読み取り確認。readonly DBはintegrity_check=ok／foreign_key_check0行、会議8／解析版8／議事録版2／回復下書き4／出力記録3。bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020`。前の追加復元用読取検査も成功（2下書き・1議事録・2編集履歴・12根拠のID対応内容、音声8SHA、PCM除外証拠2）。件数確認だけを全原本bitwise一致としない。

コピー・署名・関連プロセス不在とDirectory／所有者skodani／mode700／inode171008788を確認し、再生成可能な `/private/tmp/mon-save-dialog-build.kjzKoA` 約1.2GiBだけ回収。最新QA約37MiB、旧QA、共有Rust約8.1GiB、既存データ／ログ／画像は保持。画像 `/private/tmp/mon-global-spacing-vocabulary.jpg` と `/private/tmp/mon-global-spacing-native-settings.png`。実録音本文を含む画像は会話へ添付しない。CI・コミット／Push・新規録音・権限変更なし。全画面状態・全異常系・実IME／VoiceOver・録音／長時間・配布／正式評価等の残件は維持する。

### 同じ点検の最終バンドル

ファイル入力のラベルはOptionsModalのbody直下ではなく内容ラッパー内にあるため、実際のlabel:has(file input)へ適用対象を補正。8px間隔で上下配置し、親のgapとの二重marginを除去した。ソース点検で残っていた未確定モデル取得の隣接操作と取り込み用語ページ移動にも折り返し可能な10px間隔を追加。これらのまれな状態はsource contractであり、未確定取得を新しく起こしたり実音声を取り込んだ検証ではない。

最終699件成功（runner1.723秒）、型検査と本番ビルド成功（Vite1.55秒）、git diff --check成功。長い合成議事録の編集開始／終了も入力せず確認。主操作4ボタンは32px／12px・中央229.6875pxで一致。基本書式4ボタンも寸法は32px／12pxで一致するが、保持されたスクロール位置では画面外だったため、今回の常時画面内表示の証拠としない。

最終QA構築183.29秒。途中119.47秒との参考小計302.76秒は2回の構築で、実装やGUI確認の所要時間ではない。最終保持先 `/private/tmp/mon-global-spacing-final-qa.dhqn0l/MON QA.app`、main SHA-256 `bdb3f6b07236a310a69b1c8c92409cfb851a17126f0bea64252adb0028fdcf86`。生成元コピー一致・strict deepアドホック署名・jp.mon.desktop.qa確認。この正確なパスを冷起動し既存8会議を確認、取得定義モーダルのファイル選択ラベルが別行に配置され余白を持つこと、本文JSON欄・折り畳み説明・確認ボタンの間隔を実WebViewで確認。ファイルは選択せず、モデル取得は開始していない。Escで入口に復帰、表示設定のselect／チェック／説明／再読込の配置も再確認しEsc復帰、設定値は変更していない。

画像 `/private/tmp/mon-global-spacing-final-native-file.png`、`/private/tmp/mon-global-spacing-final-native-settings.png`。編集オプションの実機証拠は途中QAで、ファイル欄・表示設定は最終QAと区別する。合成UIの保存先API不足と異なり、Mac設定で正常なQA保存先を確認した。

終了後のreadonly検査はDB整合性ok・外部キー0行、会議8／回復下書き4／出力記録3、bootstrap SHAは上記と一致。最終QA mainと最終target関連の実行プロセス不在、生成元コピー一致と署名を確認し、Directory／所有者skodani／mode700／inode171023776の最終target `/private/tmp/mon-global-spacing-final-build.YGILIV` 1,242,392KiBを回収した。最終QA37,608KiBと途中QA、既存データ／共有Rustを保持。再生成可能な構築物の削除であり、ソース・音声・モデルは削除していない。CI・コミット／Pushなし、計画全体は進行中。

### 一覧・取り込み画面の追加点検

2026-10-11、同じ全体点検を会議一覧と取り込み画面へ広げた。合成UIの一覧で取り込みボタンと説明文の縦中央一致・gap14px、会議管理の3操作40px／13px、会議作成の入力とボタン40px／13px・縦中央一致・gap10pxを確認。追加調整が必要だったのは取り込みで、見出し間0px、入力／select47.5px、狭い既定モーダルによる操作列の折り返しを観測した。

取り込みモーダルを最大720px、内容を16px間隔の一列、見出しを14px、入力・選択欄を40px／13pxに整理した。明示キャンセル・既存の同意／保存／解析ガード・controllerは変更せず、短い内側スクロールを作らない。自動テストを追加して修正前20成功／1失敗を確認後、最終700件成功・失敗／skip0（runner1.382秒、コマンド1.576秒）。型検査＋本番ビルド5.593秒成功、Vite1.47秒はその内数。空白検査成功。

合成UIの1280×720・960×640で入力とselectは40px／13px、操作4ボタンも同じ40px／13px・同じ行の上端で一致、見出し間16px、clientWidth=scrollWidth=703px。500×640では451pxで横溢れなし、必要なボタン折り返しを保ち、内側scrollport0。長いフォームの外側dialogだけはスクロールする。500pxは製品最小960pxとは区別する。合成IPCにselect_audio_importがないため表示されたエラーは、取り込み機能の製品失敗や実取り込み成功の証拠としない。ファイル選択・実取り込みなし。

合成のエラー状態を含めない編集オプション画像 `/private/tmp/mon-spacing-overall-final.png` を保存。編集設定をEscで閉じ入口へ復帰、確認用タブを閉じviewport overrideを解除した。CI・コミット／Pushなし。

追加CSSを含むMac QA構築186.63秒成功（型検査／Swift／Rust／バンドルを含む経過、内訳の二重加算なし）。保持先 `/private/tmp/mon-spacing-import-qa.B0HkEA/MON QA.app`、main SHA-256 `611fdf0e9e3ccf7b9fba1c767332fb46f9af89dc2fe2450731e4347c6da3870c`。全コピー一致・strict deepアドホック署名成功・識別子jp.mon.desktop.qa。配布署名／公証ではなく、通知文の未承認12件・JS chunk警告等は維持する。

この正確なパスを冷起動し会議8件を確認。最初のファイル選択はキャンセルして一覧に戻ったため、その操作を取り込み設定画面の確認成功とはしない。再度、既存QA用 `/private/tmp/mon-native-ui-smoke.SOSoB9/native-smoke.wav` を選び、6秒・16kHz・mono・PCMの情報を表示するところまで進めた。取り込む／解析は開始せず、実WebViewで見出しとinfoの中央、項目間の余白、入力とselectの高さ、4操作の同じ行・高さ・文字サイズを画像で確認。数値は合成DOMの実測であり、Macでは目視確認に区別する。画像 `/private/tmp/mon-spacing-import-native-final.png`。

Esc後の「取り込み確認内容の破棄」を明示確認し、未保存の選択確認だけを取消。元ファイル／保存会議保持の通知と8件の一覧へ戻った。新しい会議・取り込み・録音・解析・設定変更なし。native AXの焦点はワークスペースcontainerだったため、この取り込み取消で入口ボタンへの焦点復帰を証明したとは扱わない。全VoiceOver／実IME・全異常系の検証も残す。

Cmd-Q後、readonly DB整合性ok／外部キー違反0行、会議8／解析版8／議事録版2／下書き4／出力記録3を保持、bootstrap SHAも上記と一致。対象QA main／専用targetの実行プロセス不在を確認。生成元コピーと署名、非リンクDirectory／uid501／mode700／inode171032968を照合した専用target `/private/tmp/mon-spacing-import-build.oBNnnT` 1,242,812KiBだけを回収し、QA37,608KiB・共有Rust・元音声／会議・画像／ログを保持。回収後のQA署名検査と空白検査も成功。CI・コミット／Pushなし、正式評価／Windows保留と計画全体の残項目は変更しない。

## 全体点検の補足：見出しと設定入力の中央揃え

追加の合成ブラウザ点検で、OptionsModalのh2下余白20pxが見出し行にも適用され、録音セッションの見出し中心219.773pxに対してinfo中心229.773pxとなる10pxずれを再現。見出し行内だけmarginを除き、節タイトルの下余白は維持した。修正後は両中心198.492px、解析工程のh3／infoも差約0.004px。見出しとinfoの間隔は8px。

エンジン設定の入力4欄は44.094pxでselect／操作40pxと不一致だったため、モーダル内の一行入力を40px・文字13px・行高20pxに統一。checkbox／radio／range／file／hidden／button／submitは除外し、複数行編集の高さを固定しない。設定fieldsetの項目間16px、ラベルと入力8px、プロファイル群の重複marginを除き、数値項目は下端揃えとした。変更はCSSとsource contract1件のみで、保存・解析・録音条件は変更しない。

CUAで1280×720と560×640の会議操作・文字起こし編集オプション・エンジン設定・表示設定・モデル取り込み入口・議事録の表示／編集／書式操作を確認。エンジンのselectと入力4欄はすべて40px／13px／20px、560pxでbodyと対象dialogの横溢れ0、内側scrollport0。編集オプションは40px／13px、区分内12px／区分境界20px。560pxの議事録書式は閉じるを含む16ボタンすべて40px／13px、横溢れ0。長いモーダルの外側スクロールは保持する。最初の閉じる操作は入れ子の2ボタンを曖昧に指定して拒否されたため、正確なdialog内へ対象を限定して検証した。初回のviewport binding参照は未定義で失敗し、既存ブラウザIDのhandleを取得してから560pxを実測した。

最新701テスト成功（runner1.464秒、失敗／skip0）、型検査と本番ビルド成功（Vite部分1.48秒、全build経過は未計測）。既存chunk警告は残る。画像 `/private/tmp/mon-spacing-heading-final.png` と `/private/tmp/mon-spacing-audit-final.png` を保存、viewport解除・確認タブ終了、ユーザーのタブを保持。本節は合成IPCのブラウザ検証であり、追加CSSは保持Mac QA B0HkEAに未反映。Mac再構築・実機再検証／Rust／CI／コミット／Pushは今回行わず、全画面状態・全異常系・IME／VoiceOverを完了扱いにしない。

## 2026-10-11 入れ子の設定・根拠パネル・狭幅の追加点検

今回の追加変更は表示CSSと回帰テスト3件のみ。既存未コミット変更を保持し、保存・録音・AI実行・確認ガードは変更しない。会議一覧、文字起こし本文、編集オプション、発言別設定、表示設定、ローカル取込、モデル配置・構成記録、録音準備（開始なし）、長い合成議事録・固定根拠、トーストをCUAで点検した。

- モデル配置記録の見出し行下端と再読込ボタン上端が一致し、間隔0pxだった。入れ子sectionにも12px gapを設け、修正後の実測12px。構成記事は8px間隔と区切りを設け、長いIDは折り返す。直下のボタンは全幅へ引き伸ばさない。
- 取込フォームは外側gridだけに依存せず、ラベルと操作に16px間隔を明示。修正後の実測16px。値・submit・disabled条件は不変。
- 議事録根拠の見出しと閉じるボタンはflex-startで約8.1pxの中心差があった。中央揃えにし、修正後の実測0px。
- 根拠の「参照記録」と直後の音声操作に間隔がなかった。12pxの余白を追加し実測12px。展開後の記録にも8px間隔を設け、隣接する操作は折り返し時も行内の高さを揃える。固定文字起こしの見出しmarginと本文／認識原文の間隔も整理。
- 600×760の文字起こしで「話者一覧を開く」がタブ・左境界に密着していた。上12px・左右16pxのinsetを追加し実測上12px／左16px、body横溢れ0。600pxは製品の最小幅960pxとは区別する。

ブラウザ実測は1280×720と600×760。狭幅の編集オプションはclientWidth=scrollWidth=566px、表示中の全button／selectは40px・13px。トーストの成功アイコン・本文・info・閉じる中心は全て35px、clientHeight=scrollHeight=40px、hostのoverflowはvisible。閉じる後のローカル取込入口へのフォーカスを確認した。合成保存先API不足による表示は実製品の失敗ではない。viewport適用は他のタブへ即時反映されなかったため、実測innerWidth=600のタブだけを狭幅検証として扱う。

最終704テスト成功・失敗／skip0（runner1.451秒）。型検査＋本番ビルド成功（Vite1.49秒、全経過未計測）、既存JS chunk警告は残る。空白検査成功。保存画像は `/private/tmp/mon-overall-spacing-model-records.png`、`/private/tmp/mon-overall-spacing-evidence.png`、`/private/tmp/mon-overall-spacing-transcript-narrow.png`。新規Rust／Mac QAの再構築は行わず、前ターンの保持QAに今回追加CSSが反映されたとは扱わない。録音・モデル操作・実データ変更・CI・コミット／Pushなし。全異常状態・IME／VoiceOver・計画全体の完了証明とは別。

## 2026-10-11 モデル検査モーダルの配置と結果導線

上記CSSを含む最新QAを構築した。手動検査結果が別の記録モーダルへ隠れていたため、入力・進捗・失敗理由・検査結果・コピー操作を同じモーダルへ配置。モーダルは最大720px、結果見出しの余分なmarginを除き、状態と次操作には12px間隔を設けた。長い説明とファイルの検査記録は折り畳み、結果と操作の初期表示を優先する。コピー後の「配置・構成記録を開く」は表示だけを切り替え、CPU実行／適用は開始しない。入力変更時は古い結果通知を消し、閉じる／開くでは入力と検査結果を保持する。処理からの自動開閉は追加しない。コピー・CPU実行・適用の個別確認と既存controllerのガードは変更しない。

CUAの合成IPC画面で検査→コピー確認取消→明示コピー→配置記録へ移動を確認。移動時のカウンターは検査1／コピー1／CPU0／適用0。CPU確認取消時もCPU0、別の明示確認による合成CPU成功後に適用操作が利用可能となることを確認した。検査失敗は同じモーダルに原因を表示し、閉じた後の入口へのフォーカスと入力保持、入力変更による古い失敗消去を確認。10秒遅延の検査取消では取消要求中も入力を無効にし、終了後に入力を戻してコピー結果を作らず、検査1／停止1／コピー0／CPU0／適用0を確認した。既存のブラウザdriverもモーダル構造へ更新したが、全driverの一括実行を今回の検証へ数えない。開発中のnative buildによるHMR再読込は合成画面を初期化したため、再読込直後の閉じる対象不在を製品不具合や成功証拠としない。

フロント自動テスト706件・失敗／skip0（runner2.248秒）。最新QA構築で型検査・本番ビルド成功（Vite1.47秒）。初回Rust release93秒、最終54.73秒。観測上の経過103.667秒／93.628秒には出力取得までの待ちが含まれ、正確な構築全体や実装所要時間とは扱わない。通常Rustテストは追加実行なし。JS chunk警告、依存通知275件のうち12件未承認、公証スキップを維持する。

最新保持先 `/private/tmp/mon-model-guide-qa.zFYwZC/MON QA.app` は生成元全コピー一致、strict deepアドホック署名検査成功、識別子jp.mon.desktop.qa、arm64 main SHA-256 `a22b8107f8115180b2ec155c063c51ce5b4df61c32ea48cee1fddbd85a808364`。読み取りプロセス照合でこの正確なパスからの起動を確認した。既存公式whisper素材 `/private/tmp/mon-managed-whisper-qa.oyBDB9` の事前検査のみ実施し、2ファイル／80,193,385bytes・未実行／互換性未検証の結果とコピー操作が同じ画面内に表示されること、長い版識別子の折り返し、項目間の余白を目視確認。コピー確認は取消し、入力と結果の保持を確認した。取消直後のnative AXフォーカスはモーダルcontainerであり、コピー操作ボタンへの復帰成功とは扱わない。Escで閉じた際はローカル取込入口に復帰し、再表示でも入力と結果を保持した。実コピー・CPU・適用はこのターンでは増やさず、次工程の表示切替は合成検証と区別する。画像 `/private/tmp/mon-model-guide-native-result.png`。

Cmd-Q後に指定QA／専用targetの実行プロセス不在を確認。readonly DB整合性ok・外部キー0行、会議8／モデル配置11／プロファイル1／CPU記録1／ジョブ6（全succeeded）を保持。ジョブ状態の初回集計は存在しないstatus列で失敗したため、table_info確認後にstateで再集計した。会議・回復下書き・ジョブ・出力記録の全行JSON SHAは起動前後とも `e6a9150d99eac460a7653a9ab9f6127975d4fb0c01ab9d1d499f8e9888cebd5f`、bootstrap SHAは `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` で一致。未集計の全テーブルまでbitwise不変とする証拠ではない。

終了・コピー一致・署名・非リンクDirectory／uid501／mode700／inode171082035を確認した専用target `/private/tmp/mon-model-guide-build.G5duYE` 1,392,460KiB（約1.33GiB）の再生成可能な構築物だけを回収。QA58,488KiB・共有Rust8,421,520KiB（約8.03GiB）・既存モデル／音声／会議／画像は保持、回収後のQA署名検査成功。録音・TCC変更・追加取得・CI・コミット／Pushなし。全異常状態・VoiceOver／実IME・配布署名／公証・長時間／正式評価等の残件は維持する。

## 2026-10-11 全体余白の再点検とモデル設定の適用範囲

文字起こしの発言設定・話者／時刻／補足、会議操作、議事録の書式操作、モデル検査・配置記録・取得定義・取得記録をCUAの合成画面で点検。議事録書式ボタンは通常幅／360px幅で40px、折り返し行の間隔10px、同じ行の上下位置一致を確認。発言メタデータは話者・時刻・複数話者表示の中心が一致していたため不要な変更はしない。

残っていた問題は、モデル設定をbodyへportalした際に、元カードの`.model-inspection`／`.model-download`を前提とした余白・枠・折り返し指定が届かないこと。SettingsDisclosureの表示専用classNameをOptionsModalの安定したhostへ渡し、手動検査／配置構成記録／取得定義／取得記録の4入口に専用scopeを付与した。既存の確認、入力保持、停止・実行・保存ガードは変更しない。

修正後の合成UIで検査結果grid／gap12px、パス入力40px・border1px、配置操作40px・同一行のy一致／間隔10pxを測定。360px幅で展開した準備説明のpre-wrapと横溢れ0を確認。取得定義はfile label gap8px、ファイルボタン右余白12px、textarea角丸8px、360pxで横溢れ0・子scrollport0。取得記録はgap16px／overflow visible（空の合成catalogで確認、長大実記録の完了証拠ではない）。合成検査1／コピー0／CPU0／適用0、実ファイル・モデル・会議・権限は変更していない。画像 `/private/tmp/mon-spacing-audit-settings-20261011.png` は合成の配置記録画面。

フロント743件成功・失敗／skip0、runner1.548秒（`/private/tmp/mon-spacing-audit-tests-20261011.log`）。型検査・本番ビルド成功、Vite1.59秒、既存の大きいJS chunk警告あり。今回の変更でnative QAの再構築／起動・通常Rustテストは行わず、以前のnative証拠を今回の変更の実機検証へ拡大しない。CI・コミット／Pushなし。全異常状態・全画面サイズ・VoiceOver／IMEおよび目標全体の残件は維持する。

### 後続のMac確認・固定hostと残る子モーダル

上記CSSを含むQAを再構築し、録音ではなくMacの音声合成で作った29.610375秒・16kHz mono PCM WAVだけを通常ファイル選択から新規取り込みした。会議名は「QA合成会議・実AI編集 2026-10-11」、ID `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3`。ファイル選択のGoTo入力は一括キー送信で意図した場所にならなかったため、各遷移のAXを確認して選択し直し、約948KBのvalid WAVを確認してから取り込んだ。初回の合成に失敗した小さいファイルは取り込んでいない。元WAVと管理original.wavのSHAはともに `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`。実ASRジョブ `2b04146d-014a-4e14-8022-2dbdeee261cf` は2495ms、実話者分離 `edf633ff-2a08-49a9-bec4-44e17a4965c2` は6556msでsucceeded。候補#1と#3（8発言・推定1話者）を保存し、現在版／議事録版はnullのまま。これは単一TTS話者の短い動作検査であり、3／10話者・精度・長時間の正式評価ではない。候補採用・本文／話者編集・議事録生成はこの追加会議では未実施。

原版QA `/private/tmp/mon-synthetic-flow-qa.OORIic/MON QA.app`（main SHA `c470b1f31b92f45972b234cb1708a7f4c973d54ac8ee87f72d90e2da0e5bccdd`）で、単独の会議操作が目視正常でもnative AXは空のHTMLcontentになることを再現。CommonModalのshowModal直前にbody hostをappendする操作だけを除いた比較QA `/private/tmp/mon-stable-modal-host-ab-qa.KdGGQN/MON QA.app`（main SHA `9a5f119757f690cc0d86593e06073c79a4a5542fb989143cce7bb84e5aae97a0`）では同じ会議操作の見出し・入力・全操作がAXへ戻り、再表示でも保持。初期焦点は閉じる、Tabは録音説明、Escは会議操作入口へ復帰した。親のnative top-layer／inertと順序管理は保持し、不要になったhost移動helperを削除。テストは入力／host保持と再表示時の順序、およびキュー許可後のshowModalと非移動を検査する形へ置換した。

子のエンジン設定は目視正常・Escで親の入口へ復帰したが、native AXは閉じるだけになり、欠落は解消していない。単独親の改善を全入れ子・全VoiceOver／IME成功へ拡大しない。比較QAはhelper削除・コメント・テスト更新の前に構築したもので、実行されるCommonModalの非移動動作とCSSは同一、最終ソースそのものの再bundleとは区別する。原版／比較版はコピー一致・strict deepアドホック署名検査成功。Cargo工程1分09秒／1分10秒、Vite1.48秒／1.50秒、Swift0.16秒／0.15秒。各工程の単純合算を全実経過にしない。公証skip・依存通知未承認12件・JS chunk警告は維持する。

最終ソースは743件成功・失敗／skip0、runner1.71679秒（`/private/tmp/mon-spacing-final-tests-20261011.log`）、型検査／本番build成功、Vite1.53秒。空白検査成功。最終合成の編集オプションは720px幅・24px内側余白・section gap12px、同一行の表示中button／selectは40px／13pxで上端一致、横溢れなし。画像 `/private/tmp/mon-spacing-final-edit-options-20261011.png`。子の閉じた回復モーダルの非表示要素は0pxなので寸法集計へ含めない。Macの親会議操作画像 `/private/tmp/mon-spacing-final-native-meeting-20261011.png` は目視証拠であり、合成DOMの数値とは区別する。

QA終了後の読み取り検査で整合性ok・外部キー違反0行、会議11（元10＋今回1）。元10件を対象にした下書き／出力JSON SHA `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、解析／議事録版＋会議現在参照 SHA `ec0e7e79ae387d584bf8df60b9fab8f1e08f5b08fd726bdd40262b5bb2925113`、app_settings＋元ジョブ SHA `7a343309ccedffe1be17416e314b965c001ebdb1396b7346dac23f776c25541c` は起動前一致。bootstrapも `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` で一致。未検査の全表・全ファイルまで不変としない。比較QA指定pathのプロセス不在を確認。共有QA targetを再利用し、大容量の別targetを追加しない。追加ダウンロード・録音・権限変更・CI・コミット／Pushなし。

## 2026-10-11 解析候補の同一境界化と実AI編集・出力

上記固定host比較QAで新しい合成会議の候補#3を開くと、プレビューのnative AXは閉じるだけになることを再現。Escで親の候補入口へ戻り、採用せず終了。解析操作controllerの状態を親へ移すのでなく、AnalysisPanelのrenderToolsへ表示だけを渡し、親OptionsModalのreplacementとして表示する。元入力とcontrollerはmountを維持し、preview／採用確認／親の単一native境界を使う。standalone表示のCommonModalはfallbackとして残す。既存の明示確認・保存待ち・期待版／下書き／hash・応答照合は変えていない。

合成CUAではreview／確認ともopen dialogが1件、取消後の採用ボタンfocus、閉じた後の候補入口focus、元のローカル用語ヒント入力保持を確認。明示採用#2→直前版#1の明示復元も確認した。これは合成UIの表示・焦点の証拠であり、IPC回数や実DB保存を推測しない。document-modal-browser-checkのreview取消期待を「全dialogなし」から「親1件と実入口focus」へ、採用後の消えた入口のfallbackを親見出しへ更新。driver構文／契約検査成功、CLI全行程は未実行で既存の他経路まで更新済みとはしない。

最終QA `/private/tmp/mon-analysis-review-qa.D4EQd5/MON QA.app`、main SHA `387a8a356bb62f15bcb4fb3c977a2057e93435e7a89b039c554ecd62d207fc5d`。同梱コピー一致・strict deepアドホック署名成功。Macで全8発言／話者／時刻／再生／採用がnative AXへ戻り、取消は採用ボタン、採用で消えた入口は親見出し、親Escは会議操作入口へ復帰。単独の親と解析reviewの限定成功で、他の子エンジン設定／全VoiceOver／IMEを合格扱いにしない。

変更した実DBデータは既に自作音声だけを入れた会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3`。候補#3 `9e45a9b0-97d1-4285-93ec-fe681f35045c` を明示採用し、本文クリック／確定で4箇所を既知の読み上げ原稿へ修正、名前ラベルから「QA読み上げ」へ変更した。下書き `dc977c24-3283-42ab-be64-f5adbd9e0508`、lockVersion5。認識原文は版に保持。00:00:07–00:00:11の保存音声を再生し0:03/0:03と再生停止を確認。Cmd-Qと指定pathのプロセス不在→冷起動で名前／保存済み／点検日検索2/8発言を確認。画像 `/private/tmp/mon-analysis-review-cold-transcript-20261011.png`。録音・マイク入力は使用していない。

同会議から生成を1回開始。ジョブ `fd25b2bc-ca71-4b06-a191-28cd2c320895` は48,747msでsucceeded、実llama-completionとQwen2.5 1.5B Q4_K_Mの固定profileVersion3を使用。新しい固定入力 `86aec1a3-ed59-400a-ab62-f6ca8f64f60e` は上記解析と下書きID／lockVersion5、内容hashを参照する。候補 `37c4cdc2-31af-4944-b966-45cb0dce8c4a` の参加者表示にQA読み上げ、決定事項・固定引用・文脈に修正後の点検日本文、認識原文を確認。実生成result.jsonも照合した。ToDoは空、担当／期限は構造化されず一部依頼が決定へ分類されているため、生成成功を抽出精度合格としない。根拠状態は未確認のまま。CPU利用でGPU非対応警告・モデルtoken警告あり。

候補#1を対象名付き確認で採用し、本文リッチ編集で会議日時段落へ「（QA確認用）」を追加、保存済みを確認。ネイティブ日本語キー入力では期待した全字が入らなかったためAXで気づき、pasteで同段落の全期待本文へ置換して保存。これは全日本語IME入力の合格証拠ではない。議事録下書き `62503797-df6c-49bd-b031-b9bfb9015c2e` lockVersion2。通常保存パネルから存在しない新しいQA専用名を指定し、`/private/tmp/mon-synthetic-flow-audio.iAJXQT/qa-edited-minutes-20261011.md` を実書き出し。本文修正・話者名・点検日・全8固定引用と解析IDを確認。2,866bytes、ファイルSHA `daa04872332a702700aae9dc98a1a23ee7d3523bb74e3fb1ab42f6030803d817` はsucceeded出力履歴 `7c78192c-f6ca-4fde-9732-e03daab6ded5` と一致。再度終了／プロセス不在／冷起動し、保存済み本文修正と出力履歴を確認。画像 `/private/tmp/mon-analysis-review-cold-minutes-20261011.png`。全書式・編集競合・復元・全話者編集は未網羅。

最終フロント746件成功・失敗／skip0、runner1.523558秒（`/private/tmp/mon-analysis-review-final746-tests-20261011.log`）。型検査／本番build成功Vite1.47秒、QA Vite1.48秒／Swift0.17秒／Cargo1分15秒（`/private/tmp/mon-analysis-review-qa-build-20261011.log`）。製品変更後745件成功とQAを構築し、後続は回帰driver／契約テスト／文書だけの変更なのでQA製品コードは同一。初回のsource契約1件はOptionsModalタグ改行への依存で失敗し、既存検査を緩めずsource整形を戻して解消。空白検査成功。JS chunk／未承認notice12件・公証skipは残る。フロント変更のみで通常Rust22分検査は再実行しない。

最終QA終了・指定pathのプロセス不在後、readonly DB整合性ok・外部キー違反0行・会議11を確認。元10件の上記保護対象3組SHAとbootstrapが一致。自作会議の文字起こし下書き全行SHA `2cba06c39434846533277190a95c7e8552994687373715138ad1624825b44ed1` は議事録生成／再起動の前後で一致。未検査の全表／全ファイルまで不変とはしない。共有QA構築target約2.11GiBと保持QA約36.7MiB、別大容量targetは作らない。ユーザー回答に従い実録音は今は実施せず、Developer ID／Team ID／公証用profileは未準備として保留。追加取得・TCC変更・正式署名／公証・CI・コミット／Pushなし。正式評価／Windows／全UI・全IME／VoiceOver・実録音／4時間・正式配布／試用と目標全体はなお未完了。

## 2026-10-11 文字起こしエンジン設定の同一境界化

残るnative AX欠落のうち文字起こしエンジン設定を対象とした。元の子OptionsModalを、会議操作のCommonModal内に常時mountする副viewへ変更。OptionsModalはCommonModalの既存persistentReplacementを受け渡し、AnalysisPanelのrenderToolsは常時保持するviewと現在の表示メタデータを親へ渡す。元フォーム・queue・profile・commandはAnalysisPanelに残る。レビュー優先で表示し、設定はhiddenで保持、standalone AnalysisPanelは従来のOptionsModalをfallbackとする。開く／閉じるは表示stateだけを変更し、保存・破棄・開始を追加していない。engine専用scopeでfieldset・labelのgap、枠線と40px入力を補正した。

合成CUAでヒント入力→engine設定（open dialog1件）→CPU6入力→Esc親入口→候補比較→採用確認→取消で実採用入口focus→プレビュー閉じるで実候補入口focus→engine再表示CPU6を確認。親モーダル全体を閉じて再表示してもCPU6を保持。合成設定の保存はfixture内のみ、実設定は変更しない。通常幅は入力5件の高さ40px・font13px・border1px、view gap16px。360×740でdialog幅328px、横溢れ／page横溢れ／内側scrollport0、通常button／select／input7件40px・13px。infoの小さい円形操作・checkboxは同寸集計から除外。viewportは終了前にreset。一時検証タブは閉じた。画像 `/private/tmp/mon-asr-engine-view-browser-20261011.png`。

通常操作だけを使う `analysis-engine-modal-browser-check.js` を登録し、親／設定／review／confirm取消／再表示を検査するdriverを追加。構文／登録／期待値のsource契約検査は成功。CUAで同じ実表示の行程を確認したが、このCLI driverそのものの全実行は行っていない。既存他driverの全面移行・全CLI成功としない。

最終QA `/private/tmp/mon-asr-engine-view-qa.E90aPz/MON QA.app`、main SHA `65e8c9da9e34e59a893635acab57ef537d2d1fdf1cad87d7cfe0e4c8d30d320a`。ビルドbundleとの全コピー一致、strict deepアドホック署名成功。新しい合成会議だけを開き、会議操作内の非保存ヒントへQA入力を入れた。engineの全パス・CPU・GPU・profile・保存状態がnative AXへ列挙され、初期焦点は閉じる。Tabでinfo→profile、Spaceで選択menu表示、高精度へ切替・Return確定、Escは親engine入口。再表示で高精度保持、標準へ戻して元の管理パス／CPU4・GPUoffを確認。menu選択が即時の差分へ出ない場面は、全AXの読み直しで標準を確認してから続行した。profileの選択はローカルstateだけで、設定queueはdirtyにならず、保存は実行しない。

infoも本文／閉じるをAXへ表示、Escでinfo入口→次のEscでengine入口へ復帰。設定を閉じて、残っている合成会議の候補#1比較を開き、採用確認は取消だけを実行、reviewの採用ボタン→親候補入口へ復帰。再度engineを開いて標準／元パスを確認し、設定→親をEscで閉じて会議操作入口へ戻り、Cmd-Q。画像 `/private/tmp/mon-asr-engine-view-native-20261011.png`。この特定画面のnative AX・キーボード成功を全VoiceOver／実IME／他の子設定・全異常時の合格に拡大しない。新機能の追加はない。

最終フロント748件成功・失敗／skip0、runner1.437289秒（`/private/tmp/mon-asr-engine-view-final-tests-20261011.log`）。先行747件も成功。型検査／本番buildは成功、先行Vite1.48秒。最終CSSを含むQA buildは終了0、Vite1.43秒／Swift0.15秒／Cargo1分12秒（`/private/tmp/mon-asr-engine-view-qa-build-20261011.log`）。後続はdriver／契約test／文書だけで製品ソースはQA同梱と同一。JS chunk、readonly Swift cache、未承認notice12件、公証skipは維持。共有targetを再利用し約2.11GiB、保持QA約36.7MiB。通常Rust全体はfrontendだけの変更なので再実行していない。

QA終了・指定pathのプロセス不在後、readonly整合性ok・外部キー違反0行・会議11。今回は全11件を含めた起動前後の保護対象SQLを照合し、下書き＋出力SHA `888868a912b99090c062f9607f5f3a6cf5057d1930359fefbfe4af235fa4c4bb`、解析／議事録版＋会議参照SHA `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、app_settings＋ジョブSHA `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226` がすべて一致。bootstrapも `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致。未集計の全表／全ファイルまで不変としない。実データ編集・設定保存・解析開始・追加取得・録音・TCC変更・正式署名／公証・CI・コミット／Pushなし。他の話者分離・用語等の子設定、全話者編集・全IME／VoiceOver・正式品質・Windows・実録音／4時間・正式配布／試用等と目標全体は継続中。

## 2026-10-11 会議別用語と話者分離設定の整理

録音は今は実施しないという回答、Developer ID／Team ID／公証profile未準備を維持し、非録音の設定UIを進めた。会議別用語はrenderEditorで親会議操作と同じCommonModalのpersistentReplacementへ描画し、用語controller・保存queue・stamp・入力破棄／削除確認は変更しない。入口のIssueNoticeは一つだけとし、補助viewは表示専用IssueStatusにして通知effectの重複を避けた。単独／共通用語の既存fallbackは残す。

話者分離は既存のstable portal／controller／countsを保ち、DiarizationPanel自身が人数画面のOptionsModalを所有する。エンジン設定は同じboundaryのpersistent bodyへ切替。Transcript側の旧OptionsModal wrapperをbare entry hostへ変更し、議事録タブでは入口とopen状態をhiddenにする。固定ジョブ条件・保存待ち・明示開始／取消／retryの処理は変更しない。詳細viewへ移したため旧diarization-panel依存CSSが届かなくなったruntime textareaの矩形枠を実画面で検出し、共通engine fieldにtextareaも含めた。字体13px／line20px／角丸8px／padding9px 12px、textarea高さ80pxに統一。新機能の追加はない。

### 合成画面で確認した範囲

CUAの一時tab36、会議A。人数を固定7に変更→詳細設定→閉じるで7を保持し、focusは詳細入口、open native dialogは1件。親を閉じる→議事録→文字起こし→人数再表示でも7を保持。会議操作→会議別用語でopen1件、空欄の新規用語に補足だけQA文字列を入力して自動保存不可の状態を作った。閉じる／再表示で補足を保持、親の用語入口にfocus復帰。再読込確認・削除確認をそれぞれ取消すると補足を保持し、実際の再読込／削除入口にfocus復帰。実ファイル・実DBへの保存ではない。

360×740の用語viewはpage横溢れfalse、dialog clientWidth／scrollWidthとも311px。可視の追加・再読込・保存・削除buttonはいずれも高さ40px／13px。小画面の話者一覧は別panelとして開くため、一覧→人数→詳細ではopen dialogは2件（一覧＋設定）、この入れ子まで同一境界化したとはしない。詳細の横溢れfalse、textarea幅263px／高さ80px／角丸8px／13px。viewportはreset、一時tabは閉じた。修正後通常幅1280pxの詳細設定画像は `/private/tmp/mon-settings-owner-browser-20261011.png`。

### Mac QAと未解決の表示順序

最終bundle `/private/tmp/mon-settings-owner-qa.BLriwP/MON QA.app`、main SHA `3db5057ac64ced1f06e15aef4aea89d4e6e8a269bf0a9b44d920086921f8660b`。構築bundleとのdiff一致・strict deepアドホック署名検証終了0。自作合成会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3` だけを開いた。人数menuで固定を選択／Returnで確定し7を入力、詳細設定の全パス・runtime・数値・profile・保存状態がnative AXへ列挙された。説明panelもAXで表示、Escは説明入口→詳細入口→人数入口と一段ずつ戻り、7を保持した。解析開始／設定保存はしない。

その後に会議操作を開くと画面は表示されてもnative AXがHTML contentだけになる欠落を再現。Escで閉じるとworkspace AXは戻るが、再表示＋Tabでも欠落が残る。冷起動して会議操作を先に開く順序ではnative AXが列挙され、会議別用語のheading・追加／再読込／保存・空状態を確認、Tabで共通適用情報のsummary、Escで親の用語入口へ復帰。Macでは新規用語入力／保存を行っておらず、合成入力保持をMac全IME／VoiceOver合格へ拡大しない。画像 `/private/tmp/mon-settings-owner-native-vocabulary-20261011.png`。

比較用の直前bundle `/private/tmp/mon-asr-engine-view-qa.E90aPz/MON QA.app` でも、人数画面だけを開閉した後の会議操作はAX正常、詳細設定→info→Escでinfo／詳細／人数を閉じる→会議操作の順序では同じ欠落を再現した。したがって今回だけの新規回帰とは断定せず、native境界をまたぐ表示順序の既存未解決として継続対応する。小画面一覧の入れ子、未採用会議のfallback／採用後のhost移動を含め、全順序／全編集／全IME／VoiceOverは未完了。Cmd-Qで両QAの終了操作を行った。プロセス不在のterminal追確認はsysmond未提供のため取得不可であり、不在を証明したとはしない。

### 検査結果とデータ保持

最終フロント753件成功・失敗0・skip0、runner1.690546542秒。`/private/tmp/mon-settings-owner-final-tests-20261011.log` SHA `86e4cad94e8f050356a887b24c2754704ea77a5c99698fbd3c2103f1940bf9ca`。追加5件は用語2件／話者分離3件のsource契約テストでありruntimeの代替ではない。型検査／QA build終了0、Vite1.49秒、Swift0.16秒、Cargo1分13秒。`/private/tmp/mon-settings-owner-qa-build-20261011.log` SHA `8960112bf78051cb50ebb671bafd33a141ba2fc920c11f15c420e29f22b69b41`。工程別実装／操作／総経過時間は未分離、単純合算を総時間としない。共有target約2.1GiBを再利用。Rust製品ソースは今回変更しておらず通常Rust全体は再実行しない。既存chunk／Swift cache／notice未承認／公証skipの警告は維持。

起動前と終了操作後で全11会議を含むsorted JSON SQLのSHAを照合し、以下4組が一致した。下書き＋出力 `888868a912b99090c062f9607f5f3a6cf5057d1930359fefbfe4af235fa4c4bb`、解析／議事録版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、app_settings＋ジョブ `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、vocabulary_entries／vocabulary_scopes／vocabulary_snapshotsをid順に取得した用語組 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`。bootstrapも `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致。起動前の会議11・既存用語4件。未集計の全表／全ファイルまで不変としない。実データ／実設定保存・解析開始・録音・追加取得・TCC変更・正式署名／公証・CI・コミット／Pushなし。正式評価／Windows／長時間実録音／配布・試用等を含む目標はactiveのまま。

## 2026-10-11 人数指定も会議操作の同一境界へ統合

前のgoal turnは用語／話者設定変更と753件検査・新しいMac順序依存の再現がありprogress。今回は順序依存を修正した。前節の未解決履歴を消さず、比較を以下へ追加する。

### 失敗比較と採用した構成

前節のbundleで、infoを一度も開かず人数→詳細→Escで詳細／人数を閉じる→会議操作でもnative AX欠落を再現した。説明popoverだけの原因とはしない。CommonModalのclosed時にhiddenを加える案は、合成の入力保持／native modality／確認取消が成功してもMacの同順序は未解消。比較bundle `/private/tmp/mon-modal-hidden-qa.bXK9xk/MON QA.app`、main SHA `4294271f5c81837bc09ac5dfaec14ca6ab1c87b4f15bc45823924faed8ed607f`、753件成功・Cargo1分11秒。このhidden変更は撤回し、最終製品ソースに残さない。

人数指定も会議操作と同じCommonModalへpersistent bodyを渡すrenderSettings構成へ統合した。DiarizationPanel／VocabularyEditorは1つずつ常時mount、人数／queue／stamp／明示開始／固定入力の処理は保持。話者一覧へは入口だけをstable mount経由でportalし、採用前のlocal hostも維持する。会議操作を開いていなくても人数replacementの明示要求で共有ownerを開き、閉じると元の話者入口へ復帰する。人数からエンジン詳細へのchild viewはModalReplacement.parentKeyを指定し、戻る際は元view・実入口・scroll位置を復帰する。元viewのbase openerを上書きせず、hidden／disabled／inert／disconnectedな戻り先は使用しない。hiddenタブへの移動はpresentation stateだけを閉じ、保存・解析・counts resetをしない。単独DiarizationPanelのfallbackは残す。新機能・解析条件変更ではない。

### 最終QAでの再現順序と範囲

最終bundle `/private/tmp/mon-shared-diarization-qa.1n7lxU/MON QA.app`、main SHA `3075261419b4fdb9f430732a3f08418d04503930c6ba778057f80fbf8b89c064`。コピーdiff一致・strict deepアドホック署名検証終了0。自作合成会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3` だけを開く。人数→詳細（全エンジン入力AX正常）→Escで詳細／人数を閉じる→会議操作で、欠落していた会議名／用語／headingを含むAXを確認。2回目はMacの人数menuで固定を選びReturn確定、7へ入力し、詳細→info→Escで説明／詳細／人数を閉じる→会議操作でもAX正常。詳細から人数へ戻る焦点は実sherpa入口、人数7を保持する。menuの差分が即時出ない場面は全AXを読んで確定後の3を確認してから7へ変更した。

続いて会議別用語→Esc親→ASR設定（whisperパス／CPU4／標準のAX正常）→Esc親／workspace→人数再表示でも7を保持。設定保存・解析開始は行わない。native画像 `/private/tmp/mon-shared-diarization-native-20261011.png`。Cmd-Q終了操作後にreadonlyデータ照合を行った。sysmond未提供のためprocess不在まで証明したとはしない。この2順序の成功を他の独立モーダルや全VoiceOver／IME成功へ拡大しない。

合成CUAでも同じshared dialog1件、詳細→人数は実sherpa入口focus／7、人数→workspaceはnative0件／人数入口focus。用語／ASR設定往復で各viewだけを同じowner内に表示。360×740の人数viewでpage横溢れfalse、dialog clientWidth／scrollWidth326px、可視select2＋指定人数input1は40px／13px。小画面の話者一覧drawerを開いてから人数→詳細はdialog2件（drawer＋shared owner）、runtimeの可視textarea263px×80px／13px、横溢れfalse。Escで詳細→実sherpa入口、人数→drawerの実人数入口にfocus復帰し7を保持、残るdialogはdrawer1件。drawerを閉じviewport reset、一時tab38を閉じる。通常幅画像 `/private/tmp/mon-shared-diarization-browser-20261011.png`。

`diarization-modal-browser-check.js` を登録し、visible操作の人数7→詳細／info→owner／用語→タブ往復の回帰driverを追加。CUAで該当行程を検証したが、このCLI driverそのものの全実行は未実施。構文／登録／安全な操作経路のsource契約検査だけで全ブラウザーgate合格としない。採用前fallback／採用後のhost移動・resize全順序／全異常操作・全IME／VoiceOverは残検査、小画面drawerの別境界も残る。

### 最終検査と保護対象

755件成功・失敗0・skip0、runner1.501248667秒、`/private/tmp/mon-shared-diarization-final-tests-20261011.log` SHA `b02addc60f68ea11b495282e0c1490732a6a94259df631c0d4cd7a77565e822f`。最初の754件実行はsource期待式が旧`!cancel`のままで1件失敗し、新しいvalid帰還先を除外する`!usable && !cancel`の契約へ更新、残るread-busy焦点ガードの検査は維持した。再試行754件は成功、driver追加後755件成功。型検査／最終QA build終了0、Vite1.56秒／Swift0.25秒／Cargo1分14秒、log SHA `5a9fe69b91b12b898a70e351090c417def46d047c15008fd8477fdd9f700b4af`（`/private/tmp/mon-shared-diarization-qa-build-20261011.log`）。QA後はtests・文書と型説明コメントだけの追加で実行処理は不変。実装／操作の個別時間は未計測、工程合計を総経過時間にしない。共有Rust targetを再利用し、通常Rust全体は再実行しない。

前節と同じsorted SQL4組を起動前／終了操作後で再計算し、下書き＋出力 `888868a912b99090c062f9607f5f3a6cf5057d1930359fefbfe4af235fa4c4bb`、版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、app_settings＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee` が全て一致。bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 一致。readonly integrity_check ok／foreign_key_check0行／会議11。集計対象以外の全データ不変まで主張しない。実録音・正式評価／Windows・正式署名／公証・試用等は残り、goalをcompleteにしない。実データ／設定保存・解析開始・追加取得・録音・TCC変更・CI・コミット／Pushなし。

## 2026-10-11 Sonnerの複数通知表示

[公式Sonner](https://github.com/emilkowalski/sonner) の配布済み `2.0.8` をnpm公式registryから固定版で導入した。インストールscriptは実行しない。React19の既存版は変更せず、lockfileの追加とoptional peer分類変更のみ。展開174,012bytes、MIT。offline notices収集は278packages／2,531,231bytesで終了0、Sonnerの `LICENSE.md` 1,070bytes、SHA `da9201378c36ffd81f51ad0b305f3f0981e90bc558789210ad65d1161eec3b7d` を収録。従来の12件の参照のみ補完と `licenseReview=not_verified` は維持し、配布の法務完了とはしない。

Sonnerは表示・動きだけを担当し、既存ToastControllerが同一イベントの重複防止、重要通知優先の3枠／待機、4秒／操作付き8秒、重要通知の無期限保持、再試行／取り消しの所有を保持する。Sonner側は `duration=Infinity`／`dismissible=false`。表示上の退場を製品側のacknowledgeへ結び付けず、待機への移動で通知を失わない。後ろに隠れたカードは表示確認扱いにせず、期限も停止する。hover／focusはスタック全体の各通知へ適用、モーダル／非表示は従来の全体停止。通常は重ねて表示し、マウス／Option＋Tで展開、重要通知がある場合は本文を隠さず展開する。

ヘッダーの専用通知行を廃止し、固定位置へ表示。12pxのヘッダー間隔、右16px、sidebarから16px以上、380px上限と16px角丸を保持する。Sonnerの600px以下の幅ルールをアプリ幅へ補正。高さが不足すると1〜3枠へ減らし、残りは待機。通知内スクロールなし。長い操作名等で大きいカードが現れても、同じ幅では最大測定値を保持し、表示数増減による再挿入の循環を避ける。

### CUAで実描画した限定範囲

開発専用 `/tests/toast-browser.html?docked=1`、実React／Controller／Sonner、ファイル・IPC・機器なし。一時tab54を使用し、viewport reset／tab close済み。初期状態へ戻すreloadを挟み、UIボタンから次を確認した。

- 4重要通知で3表示＋1待機、表示確認3。再送でも同じid／確認3のまま。閉じると未表示の1件が現れ、確認4、入力は「編集中の点検日」を保持。閉じる後は元のボタンへfocus復帰。
- 通常3件は前面1件だけ内容が可視。Option＋Tは前面の説明ボタンへfocusし3件を展開。5秒後も3件を保持。入力クリックで入力focusを奪わず折畳み、期限が再開して順次退場。別のfresh reloadでも本文のマウスクリックで3件展開を確認（LIへのfocusも発生したため、hover単独の実時間証明とはしない）。
- 保存失敗通知の×では画面の要対応／再試行を保持。再送しても復活しない。画面からの再試行1回、明示的取り消し1回、危険な操作0。モーダルを5秒開いた後も操作付き通知を保持し、Escapeで元のモーダル入口へfocus復帰。
- 960×720ライトではheader64px不変、カード幅380px／右16px、上76px、67.59375px高さ＋12px間隔。360×640ダークではheader下端73.1875px、通知上86px、x80〜344px／幅264px、最終下端312.78125px、player上540px。ページ横溢れ0、各カードの縦横scroll差0。360×320へ変更すると1表示＋3待機、下端153.59375px／player上220px。短い画面でも重要通知を破棄しない。

画像 `/private/tmp/mon-sonner-light-960-20261011.png`、`/private/tmp/mon-sonner-dark-360-20261011.png`。fixtureの操作群が通知に覆われてクリックが届かなかった一行程は除外し、fresh reloadで対象を確認し直した。これを製品の全操作／全popover／全VoiceOver／実IME合格へ拡大しない。

### Macの限定実機確認

QA `/private/tmp/mon-sonner-qa.jWUmmk/MON QA.app`（最大高さ測定値の保持補完直前）、main SHA `9d01662f11a9b19b695320a7dc67fa0da9729174cd6f96f550cd4d8c84899a3b`。strict deepアドホック署名検証終了0。自作合成会議だけの議事録を開き、OS保存パネルCancelを2回。情報通知の表示・書き出しアイコンfocus復帰、通常期限での退場を確認。2回目はOption＋T→説明入口focus→Returnで全文→Escapeで入口→Tabで×→Returnで通知のみ閉じ、Markdownアイコンへfocus復帰を確認した。音声再生・録音・保存・解析は実行しない。画像 `/private/tmp/mon-sonner-native-20261011.png`。ネイティブ複数重要通知は実イベントで発生させていない。

Cmd-Q後のprocess不在を確認してreadonly照合。11会議／integrity_check ok。下書き＋出力 `54e5ff1ce509a1d11ce1db11fd58ae5788e6f40337c050c795a19862215e94f7`、版＋会議参照 `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`、設定＋jobs `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`、用語3表 `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee` とbootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は起動前後で一致。

最大高さ保持を含めた最新フロント803件成功・失敗0／skip0、runner1.835267166秒。`/private/tmp/mon-sonner-tests-20261011.log`。新規4件はpresentation id差分／隠れたカードの可視性／高さ容量／待機重要通知の保持と安全な操作、既存ownership契約も維持。先行QAはVite1.51秒／Cargo1分11秒、終了0、ログ `/private/tmp/mon-sonner-final-qa-build-20261011.log` SHA `891b3e961ea087a231607be4e402a2a96779dfd50ccc481b6497a253d37ecda8`。`use client`のViteでの無視、既存large chunk／Swift readonly cache／未公証警告は残る。CLI browser driver4本は新しい固定stack／退出アニメーションに対応して構文検査終了0、全driverの実行ではない。共有Rust cacheを再利用し、通常Rust全体・CI・コミット／Pushは実行しない。全UI／実IME／VoiceOver、実録音、正式署名／公証等の残件は維持。

高さ測定値保持を含む最終QAも終了0、Vite1.50秒／Cargo1分10秒、`/private/tmp/mon-sonner-capacity-qa-build-20261011.log` SHA `dd47f2f659b19107bc4591fa101f62ed1573b04058ad8658ab244cbb4301d5b6`。コピー `/private/tmp/mon-sonner-final-qa.iDCUMq/MON QA.app`、main SHA `41df36f633ada7e46718bdec7fd3374c027c7024a575b665b85cede6c848120a`、strict deep署名検証終了0。冷起動→同じ自作会議→議事録→OS保存Cancel→情報通知／元の出力入口focus→Option＋T→説明入口focus→Tab→×→Return→出力入口focus復帰を再確認。画像 `/private/tmp/mon-sonner-final-native-20261011.png`。Cmd-Q後のprocess不在と、前述4組／bootstrap一致・11会議／integrity okを再確認した。最終テストログSHA `3f70063855951b986d08e2dc1445e4c3ecd8d45a5771e2b972ea6663cfc76cdd`。3回のQA構築はCargo1分14秒／1分11秒／1分10秒。実装／操作の個別時間は未計測で、構築時間の合算を総経過時間としない。共有QA target約2.2GiB、新しいRust試験cacheを追加せず、実録音・新しいモデル取得・CI・コミット／Pushなし。

# MP3・FLACの通常Mac取り込みと確認画面

2026-10-11。M3-05／M3-08／M4-01、仕様7.2・Q12・DATA-15・UX-28の限定追確認。正式評価とWindowsは保留、録音・TCC変更・CI・コミット／Pushなし。利用者の録音ではなく、今回作成した6秒の単音を使う。これは4時間・話者数・認識品質や、各形式の解析→根拠→議事録出力全経路の合格ではない。

## 素材と通常保存

素材領域 `/private/tmp/mon-import-formats-native.TyD6ai`。既存ffmpegで作成、ffprobeで形式・1ch・6.000000秒を照合した。アプリで選ぶ前の原本は以下。

| 素材 | 条件／容量 | 原本SHA-256 | 保存会議ID |
|---|---|---|---|
| QA-MP3-6秒.mp3 | MP3、48,000Hz、523.25Hz単音、72,620 bytes | b02ef8bf5c25179f5e9845568855efa9385d3959679ca8f7872364a07c48831f | f86cd528-58fb-4e7a-b474-8361721c98bb |
| QA-FLAC-6秒.flac | FLAC、44,100Hz、659.25Hz単音、83,153 bytes | 75a2635e1bb41c49e4471cf49bb8cffe9cda2f5f65977b76e265fd4cb9315384 | 422b4cd9-a6de-49b1-840b-a81398ec4fc3 |

最新focus QA `mon-preparation-focus-final-qa.EFYgac/MON QA.app` の通常「音声を取り込む」→NSOpenPanel→内容確認で、各形式・サンプルレート・1ch・6秒・必要／空き容量を表示した。MP3は初期の標準から高精度へ変更し、「未準備、音声を先に保存」の状態を確認してから保存。次のFLACでも同じ高精度を保持した。両取り込みoptionsは `high_accuracy`／会議用語空／共通版0。保存後に会議詳細へ進み、両音声を全体再生で6秒の終端まで確認。新会議のprocessing_jobsは0件で、実AIの自動実行・結果採用・生成は行っていない。

QA終了／本体不在を確認後のreadonly照合で、新しい会議はこの2件だけ（既存8＋2＝10）、integrity_check=ok／外部キー0行。管理original.mp3／original.flacの実SHAとDBのsource_hashが素材と一致した。変換audio.flacも実SHAとfile_assetsが一致：MP3由来 `b24e6369178f083ff6e1cb28be436e8e9708042319eba546ea825cfd8a132e6b`、FLAC由来 `308352e3c0c542efb9c2045bd01fd7a9eb01cc072a6bdb3a63105efe27ae2026`。変換のffprobeは各々48,000／44,100Hz・1ch・6秒。管理原本・変換・manifestは各3資産として登録。DB読取の `audio_assets` 仮名照会は表不在で失敗し、その結果を証拠には用いない。現在スキーマを調べてfile_assets／audio_import_commits／audio_import_optionsで照合した。

readonly出力 `/private/tmp/mon-import-formats-native.TyD6ai/readonly-import-evidence.json` は複数クエリのJSON配列を順に出した記録（単一JSON文書ではない）、SHA `ee685e8c592b7e9d13d16e439639fa298e5c2950ee3037e7a85acb5dd3cc7dde`。

## 冷起動と保持境界

コンパクトQA `/private/tmp/mon-import-compact-qa.hAqu1P/MON QA.app`（main SHA `87eb57c4a5bc2472e1b175d1cebdc2fe836ab26b92b43a3688157381025e7a8a`）へ冷起動、10件の一覧を確認。Cmd＋F→検索入力→Tab2回→会議名buttonのnative焦点→ReturnでMP3／FLACの各詳細へ移り、各々0:06／0:06、再生位置288,000、終端の「再生」に戻ることを確認した。これは冷起動後の通常管理音声再生であり、VoiceOver全操作の合格ではない。AX cellへのクリックは名前buttonを押さず一覧に残ったため、成功経路へ数えない。

今回の起動前後で、全下書き＋出力記録SHA `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c` が一致。全解析／議事録版＋新しい2件を除く元8会議のtitle／current／previous参照を順に出したSHA `9dc93ef5c00733c83fc3ee2bc6b1fd7b586e26b086ed814d9287ab89a771ec00` も一致。bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` が一致。全DBが不変とは言わず、2件の保存は意図した追加として区別する。

原本の場所への依存を別に確認するため、アプリ終了後、今回所有する素材だけを同じ領域の `QA-MP3-6秒.source.mp3`／`QA-FLAC-6秒.source.flac` へ改名した。アプリの取り込み処理による変更ではない。元の選択パスを不在にし、削除せず復旧可能な形で保持する。利用者の音声・管理original／audioには手を加えない。この条件の再生追確認は最終QAで別に記録する。

## 確認画面の整理

通常Mac画面で旧取り込み情報が1列のdt→ddを繰り返し、会議名と操作が画面外へ押し出されたことを再現。情報6組は削らず、通常幅では116pxのラベル＋値をbaselineで揃える2列、行間6px・列間16pxにした。480px以下はラベル／値を4px、次の組を8px離して1列へ戻す。会議名と解析プロファイルは600px以上で2列の同じ入力行、狭幅では縦配置。両入力は40px／13px／行高20px。

重複した解析／用語見出しだけの行を整理し、解析infoは対応するselectラベルの隣、用語infoは用語モーダル入口の隣へ移した。会議名ラベルとinfo付きラベルは同じ32pxの高さ。選択欄はhtmlFor／idで元の名前「解析プロファイル」を保持し、説明の文章・同意案内・必要容量・AI未準備／候補扱いの条件を削らない。任意の用語設定はモーダルのまま、閉じたinputの安定所有・選択token・同意／保存guardを変更しない。入口だけのanalysis-settingsに残るカード余白・borderを解除した。

最終前のフロント738件成功・失敗／skip0（1.398秒）、型検査／build成功（Vite1.43秒）、Mac QA Cargo1分09秒・Swift0.15秒。コピー全一致・strict deepアドホック署名成功。配布署名／公証ではない。最初のCSSのみのフロント／単体buildも終了0だが、後のJSX整理を含む最終証拠と混同しない。

合成1280×720では情報欄162.5625px、入力の上端470.140625pxが一致、各操作40px／13px、横溢れ0・内側scrollport0。短い720px高では外側dialogに28pxの必要スクロールがある。360×640では328px幅・左16px、入力40px／13px、操作2行に10px行間、横溢れ0・内側scrollport0。320×640でも288px幅・左16px・横溢れ0。viewport overrideは解除した。fixtureのwindow.monImportFixtureを隔離された読取evaluateから直接数える試行はundefinedとなったため、要求数の証拠には使わない。DOMで会議名・profile保持と焦点復帰を確認。

コンパクトQAの実1280×820画面では情報・入力・用語入口・全4操作がスクロールなしに表示された。画像 `/private/tmp/mon-import-formats-native.TyD6ai/compact-native-confirmation.png`。両infoの開閉／Escで同じ入口に戻る。取り込み取消の確認をEscで辞退すると、選択音声・会議名・標準profileを保持、再び取消して明示実行すると元の一覧へ戻った。取り込み開始は行わず、追加会議は作らない。

## 未解決の子モーダルと入口focus

同じMacでは「会議の用語（任意）」の子モーダルは画像上の本文・閉じる・追加ボタンが正常でも、native AXは親の「確認をキャンセル」だけになった。Tabでも列挙は改善せず、Escで親と用語入口の焦点へ復帰。ブラウザーで同じ子モーダルは列挙／Esc復帰したが、Macの代替証拠にしない。原因・VoiceOver／全順序合格は未証明。

安定portal hostのdisplay:contentsだけを、余白／border0のblockへ替えた比較QAを作った。`/private/tmp/mon-import-host-block-ab.X3StyX/MON QA.app`、main SHA `dab7b65c1684d3f6a7cf8fff73bf64642562a24c194dbfe57c42d9c73240141a`、739件成功、Cargo1分08秒／Vite1.46秒／Swift0.16秒、コピー／アドホック署名成功。この比較でも同じAX欠落が再現したため、display変更とその契約期待だけを元のcontentsへ戻した。不成功の試行を恒久修正や合格にしない。

別に、最初の取り込み入口でMacのpointerがbuttonをfocusせず、取消後の焦点がworkspaceへ戻る問題を確認。入口のonClickでbuttonをfocusしてから既存selectを呼び、NSOpenPanel前のsourceOpenerを正しく捕捉するだけに変更した。比較QAで取消後に「音声を取り込む」buttonへ戻ることをnative確認。新しい契約1件を追加。ドラッグの所有者や選択・保存条件は変えない。

## 最終contents版と原本移動後の追確認

739件成功・失敗／skip0（1.424秒）、型検査／build成功。log `/private/tmp/mon-import-compact-final-v2-tests-20261011.log`（SHA `f70e79f647ce2fce5d79f71354a31e04a1b2dec122fa285134fb7d56b987ef22`）。最終QA Cargo1分08秒・Vite1.46秒・Swift0.15秒、build log `/private/tmp/mon-import-compact-final-v2-build-20261011.log`（SHA `a993df17d69dfb995639b52ad8b77d9c28bee507559b2cd047eba59050959d55`）。QAは `/private/tmp/mon-import-compact-final-qa.WMCvJP/MON QA.app`、main SHA `622b26a86e8a064b3dcba3b902151da0c22d8a520f4b81d66dac9731c9a8642b`。コピー全一致・strict deepアドホック署名成功、署名／公証配布の証拠ではない。既存の2.1GiB build targetを再利用、新しいRust大容量targetを作らない。Rust製品4ソースは最新1,829件実行時のSHAと一致、フロントだけの変更でその22分検査を再実行していない。

原本の旧2パスは不在を確認したまま最終QAへ冷起動。Cmd＋F・検索・Tab・Returnで元の保存名の2会議を開き、管理音声再生を各々0:06／0:06・288,000の終端まで確認した。FLAC画像 `/private/tmp/mon-import-formats-native.TyD6ai/final-managed-playback-after-source-move.png`。初回の検索後Tabをまとめた取得はfocus行がなかったため移動合格へ数えず、Cmd＋F→Tab→各最新stateで再読込／会議buttonの焦点を確認してからReturnした。元の選択パスへの依存がない通常再生の限定証拠であり、原本移動後の実AI再解析は今回は行わない。

最終QAで改名後のFLAC素材をもう一度内容確認まで選択し、画像 `/private/tmp/mon-import-formats-native.TyD6ai/final-compact-native-confirmation.png` に同じコンパクト配置・情報6組・40pxの入力／全4操作を保持。今回の「取り込む」は実行せず、Esc→明示的な確認内容の取消→音声を取り込むbuttonのnative焦点復帰を確認して終了した。新しい会議を重複作成しない。

Cmd-Q／QA本体不在後のreadonly照合でintegrity_check=ok／外部キー0行／10会議／新2件のprocessing_jobs0件。上記保護対象3組SHAは全て一致し、改名素材のSHAも元の値と一致。素材はsource名で保持し、削除しない。本番領域・モデル設定・既存候補の採用／生成／録音は変更していない。最終diff検査も成功。

残るM3-05条件は、ドラッグの現Mac通常経路、各形式から実AI解析／根拠／出力の全経路、4時間素材。10話者と正式品質は保留。実録音・サービス・配布／先行試用等の目標全体は未完了。ここまでの子モーダルAX欠落の記述は以下の追確認前の状態である。

## 追確認：用語設定を同じネイティブ境界へ切り替え

2026-10-11。親のnative dialogだけを一時的にcloseし、入力を保持して子へ切り替える比較実装を試した。合成ブラウザーの多階層・入力保持・確認取消・待ち合わせは成功したが、Macでは子が画像上正常でもHTMLのnative AX内容が空になり、Tabでも改善しなかった。比較QA `/private/tmp/mon-nested-modal-final-qa.AEUDbx/MON QA.app`（main SHA `1365229a92f39df5239816314ee7bd71cd90dfb542618a4ae266157fabae4ad9`）。この比較の742件成功を実機合格とせず、suspension registryとキュー／開閉変更、その専用契約を全て撤回した。既存のnative入れ子・他設定の制御は変更しない。二つのnative dialogだけが原因という推測も確定しない。

代わりに、取り込みの用語設定だけを既存CommonModalのreplacement機構で同じ境界内へ表示する。新しい任意のpersistentReplacementは一つの補助viewの入力を常時mountし、非表示時と確認中はhiddenにする。通常のreplacement利用者は従来どおり。取り込み本文もhiddenで保持、用語の閉じる／Escは元の確認へ戻り、取り込み自体を取消・保存・解析しない。500件上限、50件ページ、空欄／重複検証、入力・token一致の削除確認、保存開始guardは保持。用語fieldsetも既存のbusy／録音／保存済みロック条件を適用する。用語入口は現在のdialogをaria-controlsで参照し、同じ入口へのfocus復帰を指定する。

入力のnative既定2px borderで42pxになっていた箇所を1px・9px padding・13px／20pxへ揃え、チェック以外のinputとテキストボタンを40pxにした。labelはmargin0／gap8px、用語fieldsetは通常幅も一列。textareaの複数行・縦resizeとチェックのコンパクト寸法を維持。合成360×640ではdialog328px・左16px、320×640では288px・左16px、両方で横溢れ0・内側scrollport0・可視input／ボタン40px。viewport overrideを解除した。

合成の制御されていないinputへ「点検日・入力保持」を入力し、同じ境界での変更確認取消→閉じる→再表示でも保持。defaultValueへ戻っていないことをDOM propertyとAXで確認した。実取り込みfixtureでも用語・読み、削除確認取消と一境界・非表示の元入力の保持を確認。既存の文字起こし編集オプションも通常表示・Escで入口へ戻る。追加したbrowser-check手順全体を独立runnerで再実行したという主張ではなく、CUAで同じ操作を検査した証拠である。

中間の同一境界QA `/private/tmp/mon-import-persistent-qa.R4cAGg/MON QA.app`（main SHA `fb19e864f2ab6cc0d93bf191e72dad25326e8586df927c0bd685bc734a0ce221`）で用語見出し・追加・全入力のnative AXが表示され、入力後の閉じる→再表示も保持した。寸法修正後のQA `/private/tmp/mon-import-vocabulary-final-qa.QQTjmS/MON QA.app`（main SHA `41148c538ab41a6a2148fa32a549662317e21c36361b8252502a91197702a01b`）でも同じ成功。ただしpointerによる削除確認取消では焦点がdialogへ戻った。削除ボタンを明示openerとして既存確認へ渡す追加修正を行い、以下の最終QAで検証した。

最終QA `/private/tmp/mon-import-vocabulary-return-qa.1wKvi8/MON QA.app`、main SHA `d62f1fc29eb5d18c085224fbdaf79acb508128993a926d709a055d73f9a094da`。ビルド原本との全コピー一致、strict deepアドホック署名成功。正式署名／公証済み配布物ではない。フロント742件成功・失敗／skip0、1.632秒。型検査／build成功、Cargo1分10秒・Vite1.51秒・Swift0.16秒。途中比較の成功件数を合算しない。最終logのSHAは tests `bc92d0ba9d54bfc3c6ad1580da886f71a5b77cc4ecdde94ba96630329cbdf5b5`、QA build `ef76ac8fe3a1380e520cf088c451edaca8517d60a143b322363909a0a7c64af8`。それぞれ `/private/tmp/mon-import-vocabulary-return-final-tests-20261011.log`、`/private/tmp/mon-import-vocabulary-return-final-qa-build-20261011.log`。同じ約2.1GiB targetを再利用し、Rust製品ソースを変更していないため22分の通常Rust全体は再実行していない。

最終Macで合成FLACを通常選択し、取り込み開始は押さない。用語の安全な閉じるへ初期focus、Tab→Returnで追加、Tabで用語・読みへ移動して「点検日」「てんけんび」を入力。削除確認は安全なキャンセルが初期focus、Escで取り消し、native AXが同じ「用語を削除」buttonへ戻る。用語／読みも残る。Escで元の用語入口へ戻り、会議名を「QA用語保持・未保存」へ変更後の用語再表示・再度Escでも全入力を保持した。取り込み全体の取消確認を明示実行し、元の「音声を取り込む」へfocus復帰、Cmd-Qで終了。公開可能な合成入力だけの実機画像 `/private/tmp/mon-import-vocabulary-native-final-20261011.png` を保持。

QA本体のpgrepが終了1で不在確認後、readonly照合：integrity_check=ok／外部キー0行／10会議／先行MP3・FLACのprocessing_jobs0。全下書き＋出力記録SHA `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、全解析版＋議事録版＋全10会議title／current／previous参照SHA `ec0e7e79ae387d584bf8df60b9fab8f1e08f5b08fd726bdd40262b5bb2925113`、bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は今回の起動前と一致。取り込み・保存・AI解析・用語削除の実行、新会議の作成、本番領域の操作はない。

合格範囲は取り込み内の一つの用語viewと上記操作であり、他の入れ子設定・全IME／VoiceOverの合格に拡大しない。各形式の実AI経路・実ドロップ・4時間、実録音・配布・試用は残る。正式評価／Windowsは保留のまま、CI・コミット／Pushなし。

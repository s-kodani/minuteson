# ネイティブ依存物の通知と再配布条件

確認日: 2026-11-13（日本時間）。調査対象はリリース候補`abaad2b56be302b22d31b2735d2499b3f27a9d29`、[run 43297943714](https://github.com/s-kodani/minuteson/actions/runs/43297943714)。公開済みStore版の実際の配信ファイルとの同一性は別途確認する。

## 今回の修正

- macOSで通知本文が欠けていた12件には、版固定の上流全文を補完する。MITのみのblock2・objc2・objc2-encode・objc2-foundationはMITを維持し、他8件は宣言内のZlibを選ぶ。以前のSteven SheldonのMIT通知も保持する。
- WindowsにはLLVM OpenMPの全文とMicrosoft CRTの再配布条件への導線を追加する。アプリのクレジット用と、モデルに付随するruntime.tar内の両方に同梱する。EXEとMSIXで同じ資料を使う。
- libomp.dllは上流アーカイブSHAとDLL自体のSHAを固定する。バイトが変わったら、版と条件を確認するまで生成を失敗させる。CRTとlibompは独自の再署名対象から外し、元のバイトを保つ。
- 取得元、実際のCRTバイトのSHA、libompの版根拠を`native-dll-provenance.json`へ記録する。通知本文の有無と、追加著作権・SDK・利用権の確認を分離して記録する。

## macOSで確認できたことと残る判断

上流は[ee9a7ad](https://github.com/madsmtm/objc2/commit/ee9a7ada2131f5944b8750428e265c15632f2a19)で全文を追加した。以前の固定版の宣言はそのまま保持し、後からライセンスを変更したものとして扱わない。[上流issue #826](https://github.com/madsmtm/objc2/issues/826)では、他の寄稿者の著作権表示が十分かについて確答されていない。同じリポジトリで以前配布されていた[Steven Sheldonの通知](https://raw.githubusercontent.com/madsmtm/objc2/9961247c1a82027d6edbe6c516011b1363b9354c/LICENSE.txt)もそのまま収録する。

実際の対象crate12件のアーカイブSHAとVCS情報を照合して補完処理を実行したところ、全文欠落は0件となった。ただしMITのみの4種類の追加著作権表示の網羅性と、Apple SDK由来の配布判断は未確定である。`unresolvedNoticeReviews`には対象12件の未解決理由が残り、アプリの通知文にも表示される。全文が揃ったことだけで配布承認にはしない。390件全体の通知生成や最終DMGの再作成は今回の静的確認に含まない。

## Windows DLL

| 対象 | 確認した根拠 | 修正と未確認事項 |
| --- | --- | --- |
| libomp.dll | LLVM OpenMP Runtimeの製品情報、埋め込みパス`llvm_package_20.1.8`、Clang 20.1表記。元DLLのSHAは`a12116ba72d1d6820407cf30be23da04ce79d6bb8a71a5ee71759c5a1faa6f1c` | LLVM 20.1.8のOpenMP LICENSE.TXTを旧来の通知も含め全バイト保存。版の特定はDLL内の情報と上流配布物のハッシュに基づくもので、LLVM公式バイナリとの同一性を主張しない |
| msvcp140.dll、vcruntime140.dll、vcruntime140_1.dll | 調査した配布物ではPE版情報が14.44.35211.0。生成処理の取得元はVisual Studio 2022のVC/Redist/MSVC配下のx64 CRT | 取得元と未変更バイトを記録。REDIST一覧上の対象であることと、配布者が該当Visual Studioの有効な利用権・条件を満たすことは別。後者は未確認として保持 |

LLVMの条件は[固定版のOpenMP LICENSE.TXT](https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-20.1.8/openmp/LICENSE.TXT)に従う。動的DLLの配布を、コンパイル時に埋め込まれるコードへの例外だけで免除扱いにしない。

Microsoftの[REDIST一覧](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution#visual-c-runtime-files)は、所定の条件の下でVC/redist配下のファイルを未改変でプログラムと配布することを認め、debug_nonredist等を除外している。[再配布説明](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files)は有効にライセンスされたVisual Studioとその条件を求める。個人開発者・Community版が一律に対象外という意味ではない。

今回取得・確認したRuntimeの利用者向け条件とBuild Toolsの条件には、今回のDLLの再配布権を単独で証明できる記述は見つからなかった。実際のエディションの契約は[Microsoftのライセンス一覧](https://visualstudio.microsoft.com/license-terms/)から確認する。CIにファイルがあることだけを、配布者の利用権確認として扱わない。追加の承諾操作や契約締結は行っていない。

## 検証と次の対応

[検証記録](review-2026-11-13.json)に対象版・SHA・結果を記録する。12件の実crate照合に加え、元のWindows runtime.tarを実行せず改修した生成処理で再構成し、4 DLLが元のバイトのままであること、通知がクレジット用とモデル用に入り、カタログのハッシュ対象になることを確認した。アプリのインストール・実行を伴う検証とは区別する。

未確認の追加著作権・SDK・Visual Studio利用権は残す。`licenseReview: not_verified`と`distributionReady: false`を維持する。この変更は公開済み1.0.0やGitHubの下書きの添付物を置き換えない。マージ後に新しい候補を作る場合は、同じソースからのビルド、署名・公証、EXE／MSIX生成と該当する実機確認が必要となる。Store版へ反映する場合は公開済み版への更新として版数も判断する。

`RELEASE_PUBLISH_ENABLED=false`、`STORE_PUBLISH_ENABLED=false`を維持する。

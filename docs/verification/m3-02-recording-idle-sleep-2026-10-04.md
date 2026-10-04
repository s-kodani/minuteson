# M3-02 macOS録音中の自動スリープ抑止

仕様5.6・Q15に従い、macOS録音プローブに録音専用の電源アサーションを接続した。Windows固有部分と正式評価は保留を維持する。

## 実装

録音開始の処理内で、`IOPMAssertionCreateWithName`を使い`PreventUserIdleSystemSleep`だけを取得する。画面消灯を抑止する種類や、一般のシステムスリープを抑止する種類は取得しない。Appleの[電源アサーション定義](https://github.com/opensource-apple/IOKitUser/blob/master/pwr_mgt.subproj/IOPMLib.h)では、この種類は無操作による自動スリープだけを抑止し、画面消灯を許可する。明示スリープ・蓋閉じ・電源状態による強制中断を防ぐ保証はしない。

所有者は録音プローブであり、Tauriのメイン画面・ミニパネル・文字起こし・議事録生成ワーカーにはアサーションを作らない。アプリ起動・一覧取得・解析だけでは取得しない。録音処理が終わった後、最終manifest保存前に解除する。開始失敗・録音停止失敗・CAF確定失敗の経路でも解除し、manifest書込みの例外時には`defer`、所有オブジェクトの破棄時には最終の解除を試みる。

取得失敗時は成功や抑止中を装わず、録音を開始せず失敗manifestへ記録する。解除失敗時は所有IDを失わず、後続のcleanupが再試行できるようにする。解除失敗もmanifestの失敗として表示対象へ残す。解除成功後の重複呼出しはOSへ二重解除しない。

manifestに`power`を追加し、種類・画面消灯許可・取得/解除の結果を記録する。既存録音manifestのスキーマやRustでのトラック検証は変更しない。メイン画面を閉じても録音プローブが生存する既存経路で、抑止も録音処理に従う。

## 検証

SwiftPMのテストターゲットを追加し、`make check`から実行する。5件を実行し、取得なしの初期状態、取得/解除の冪等性、取得失敗後の再試行、解除失敗後に所有IDを保ったcleanup、例外経路の解除を確認した。

5件中1件は実macOSのIOKitを呼び出した。所有アサーションの種類・有効レベル・名称を`IOPMAssertionCopyProperties`で照合し、`IOPMAssertionRelease`後に同じIDの情報が消えることを確認した。録音権限・録音音声・端末のスリープ設定変更を必要としない。テストの抑止は短時間で解除済み。

```sh
cd probes/macos-capture
env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache \
  CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --disable-sandbox
```

最終の`make check`が成功。Python 65件、フロントエンド35件、Swift 5件、Rust通常141件（環境依存6件はignored）、AVFoundation製CAF読出し1件。正式評価の前提不足7警告は未解消として保持する。macOS開発用`.app`のビルド（16.47 MiB）と`codesign --verify --deep --strict`も成功した。署名はアドホック、公証なしで、配布・ネイティブ画面・実録音の統合確認を証明しない。

## 残件

実録音での開始・停止・メイン画面を閉じる操作に合わせた取得/解除、開始/停止失敗の実機再現、録音中の実際の無操作スリープ抑止、明示スリープ/蓋閉じ後の欠損・復帰表示は未検証・未完成。今回のAPI照合を実録音や4時間の電源・音声安定性試験の代わりにはしない。既存の録音プローブの親アプリ喪失時停止は維持するが、解析子プロセスの親アプリ強制終了対策は未実装のまま。

この追加だけでM3-02全体を完了扱いにしない。実録音は周囲とPCの音を保存するため、明示的な確認を得てから行う。Windowsの電源制御・CI再開は行っていない。

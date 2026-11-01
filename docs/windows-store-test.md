# Store提出原本と同じ内容のローカル検証

アプリ1.0.1／MSIX1.0.1.0から、Store原本と手元の検証版は同じ版数・同じ全ペイロードを使う。未署名原本をコピーして検証用に署名するだけで、マニフェストの書換え・再パッケージは行わない。正式提出には未署名のStore原本を使う。

## 準備

ビルドが完了した変更のないworktree、その完全なコミットSHA、`build.json`の絶対パスを指定する。以下のプレースホルダーは実際の完了済みビルドに置き換える。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/prepare-windows-store-test.ps1 -BuildWorkspace '<ビルドworktreeの絶対パス>' -BuildManifest '<完了したbuild.jsonの絶対パス>' -ExpectedCommit '<完全なコミットSHA>' -StorePackage '<公開用未署名MSIXの絶対パス>' -CreateCertificate
```

`-CreateCertificate`は初回のみ指定する。以前の一時証明書E4B301…／32A242…は秘密鍵を削除済みで、公開証明書から再署名することはできない。そのため次回だけ新しい検証用証明書が必要になる。

新しい検証用証明書はCurrentUser\Myに非エクスポート可能な鍵として90日保持する。名前・Publisher・署名用途・有効期限・鍵の非エクスポート設定を確認し、以後のビルドで再利用する。二回目以降は`-CreateCertificate`を省く。候補が複数ある場合は自動選択せず、`-CertificateThumbprint '<完全な拇印>'`で指定する。失敗時も鍵は自動削除しない。

準備処理は本番のソース照合を`--require-current-source`と同じ条件で実行し、原本のSHA-256、全同梱ファイル、コピー前後の一致を確認する。署名後はCMSの署名者・公開証明書・署名対象のカタログ／ブロックマップ／Content Typesを照合し、原本の全ペイロードが同じバイト列のままであることを再確認する。SignToolが追加する`AppxSignature.p7x`と`AppxMetadata/CodeIntegrity.cat`だけを署名用追加物として認める。

出力は`output/windows-store-test/<準備ID>/`。`test-manifest.json`の`payloadChanges`は空、`originalVersion`と`testVersion`は一致する。原本は変更しない。秘密鍵やPFXは出力しない。

## 手元で使う場合

準備処理は証明書の信頼登録、インストール、アプリ停止、データ変更を行わない。署名の暗号学的検証とWindowsの信頼確認は別で、準備直後の`windowsTrustVerified`は`false`、実表示は`pending`となる。

初回の新しい公開証明書だけを、拇印を確認してLocalMachine\TrustedPeopleへ手動登録する。同じ永続検証鍵を使う次回以降は通常この再登録は不要。有効期限・鍵の紛失・証明書変更時は改めて確認する。これはStoreの正式署名証明書ではない。

アプリ内で保存して「終了」した後、同じWindowsユーザーで署名済みコピーを`Add-AppxPackage -Path '<署名済みコピー>' -ErrorAction Stop`により通常更新する。強制終了・アンインストール・リセットでエラーを回避しない。スタートメニューから起動し、アイコンと既存会議・設定・モデルを確認する。

Storeで再署名された同一版への切替は、ローカル検証だけでは保証できない。Windowsが同一版の異なる署名を拒否した場合は、バックアップを確保して別途切替を計画するか、後続の高いStore版へ更新する。準備処理はStore申請・認定・正式配信の検証を含まない。

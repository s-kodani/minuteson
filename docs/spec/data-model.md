---
type: "Specification"
title: "データモデルとER図"
description: "会議・音声・解析・ジョブ・登録した声・用語の主要テーブルと関連。"
tags: ["storage", "schema", "er"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/storage.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0004_audio.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0005_content.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0010_draft_history.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0016_vocabulary.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0031_minutes_markdown.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0032_voice_identity.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0033_speaker_completion.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0036_post_analysis_intents.sql"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/migrations/0037_pcm_audio_import.sql"
---

# データモデルとER図

DB schema 38の主要テーブルを領域別に示します。テーブル・列は物理名で、PKは主キー、FKは外部キー、UKは一意制約です。関連線は実際の外部キーに基づき、`||`は1、`o|`／`|o`は0または1、`o{`は0以上を表します。実線は親のキーを子の主キーに含む識別関連、点線は非識別関連です。

列は代表項目に絞り、複合外部キーの会議ID・解析版IDの照合、CHECK制約、索引、トリガー、補助台帳は省略しています。完全な定義は[マイグレーション](../../app/src-tauri/src/migrations/)を参照してください。ファイルの所有・保存・復旧の動作は[保存仕様](storage.md)が正本です。

## 会議と音声資産

```mermaid
erDiagram
    meetings ||..o| recording_sessions : "meeting_id"
    meetings ||..o| audio_imports : "meeting_id"
    meetings ||..o{ audio_tracks : "meeting_id"
    meetings ||..o{ file_assets : "meeting_id"
    recording_sessions |o..o{ audio_tracks : "session_id"
    audio_imports |o..o{ audio_tracks : "import_id"
    audio_tracks ||..o{ audio_chunks : "track_id"
    file_assets ||..o| audio_chunks : "file_asset_id"

    meetings {
        TEXT id PK
        TEXT title
        TEXT lifecycle
        TEXT source_type
    }
    recording_sessions {
        TEXT id PK
        TEXT meeting_id FK,UK
        TEXT state
        TEXT capture_mode
    }
    audio_imports {
        TEXT id PK
        TEXT meeting_id FK,UK
        TEXT original_asset_id FK
        TEXT converted_asset_id FK
        TEXT state
    }
    audio_tracks {
        TEXT id PK
        TEXT meeting_id FK
        TEXT session_id FK
        TEXT import_id FK
        TEXT kind
    }
    audio_chunks {
        TEXT id PK
        TEXT track_id FK
        TEXT file_asset_id FK,UK
        INTEGER start_us
        INTEGER end_us
    }
    file_assets {
        TEXT id PK
        TEXT meeting_id FK
        TEXT relative_path
        TEXT content_hash
        TEXT state
    }
```

会議には録音または取り込みの由来を持たせます。各トラックの`session_id`と`import_id`はどちらか一方だけを指定します。`audio_imports`の原本・変換済み資産も`file_assets`を参照します。チャンクと管理ファイルは一意に対応し、ファイルの実体はDB外の管理領域に置きます。時計対応・欠損は`track_clock_segments`・`audio_gaps`に別途記録します。

## 解析と編集と議事録

```mermaid
erDiagram
    meetings ||..o{ analysis_revisions : "meeting_id"
    analysis_revisions ||..o{ speakers : "analysis_revision_id"
    analysis_revisions ||..o{ utterances : "analysis_revision_id"
    utterances ||--o{ utterance_speakers : "utterance_id"
    speakers ||--o{ utterance_speakers : "speaker_id"
    meetings ||..o{ working_drafts : "meeting_id"
    analysis_revisions |o..o{ working_drafts : "base_analysis_id"
    working_drafts ||..o{ edit_operations : "draft_id"
    meetings ||--o| minutes_markdown : "meeting_id"

    meetings {
        TEXT id PK
        TEXT current_analysis_id FK
    }
    analysis_revisions {
        TEXT id PK
        TEXT meeting_id FK
        INTEGER revision_no
        TEXT publication_state
        TEXT content_hash
    }
    speakers {
        TEXT id PK
        TEXT analysis_revision_id FK
        TEXT display_name
    }
    utterances {
        TEXT id PK
        TEXT analysis_revision_id FK
        TEXT track_id FK
        TEXT effective_text
    }
    utterance_speakers {
        TEXT utterance_id PK,FK
        TEXT speaker_id PK,FK
        TEXT analysis_revision_id FK
        TEXT assignment_source
    }
    working_drafts {
        TEXT id PK
        TEXT meeting_id FK
        TEXT base_analysis_id FK
        TEXT target_kind
        INTEGER lock_version
    }
    edit_operations {
        TEXT id PK
        TEXT draft_id FK
        INTEGER sequence
        TEXT operation_kind
    }
    minutes_markdown {
        TEXT meeting_id PK,FK
        TEXT markdown
        TEXT content_hash
        INTEGER lock_version
        TEXT operation_id
    }
```

解析版は発言と話者を保持し、`utterance_speakers`で一つの発言に複数話者を割り当てます。会議の`current_analysis_id`が現在版を指し、手動編集中の内容は`working_drafts`と`edit_operations`で管理します。発言は元のトラックも参照します。

現在の議事録の正本は会議ごとに最大1件の`minutes_markdown`です。DBには過去の構造化形式の`minutes_revisions`・`minutes_items`・`evidence_links`も残りますが、現行UIの議事録版復元や根拠管理を表すものではありません。[議事録仕様](minutes.md)を参照してください。

## ジョブと後続処理の意図

```mermaid
erDiagram
    meetings ||..o{ processing_jobs : "meeting_id"
    processing_jobs ||..o{ job_checkpoints : "job_id"
    meetings ||..o{ post_analysis_intents : "meeting_id"
    analysis_revisions ||..o| post_analysis_intents : "analysis_id"
    analysis_revisions ||..o{ post_analysis_intents : "current_analysis_id"
    processing_jobs |o..o{ post_analysis_intents : "completion_job_id"
    processing_jobs |o..o{ post_analysis_intents : "matching_job_id"

    meetings {
        TEXT id PK
    }
    analysis_revisions {
        TEXT id PK
        TEXT meeting_id FK
    }
    processing_jobs {
        TEXT id PK
        TEXT meeting_id FK
        TEXT operation_id UK
        TEXT kind
        TEXT state
        TEXT input_analysis_id FK
        TEXT output_analysis_id FK
        TEXT spec_hash
    }
    job_checkpoints {
        TEXT id PK
        TEXT job_id FK
        INTEGER sequence
        TEXT checkpoint_hash
    }
    post_analysis_intents {
        TEXT id PK
        TEXT meeting_id FK
        TEXT analysis_id FK,UK
        TEXT current_analysis_id FK
        TEXT completion_job_id FK
        TEXT matching_job_id FK
        TEXT phase
    }
```

`processing_jobs`が固定入力・実行条件・操作IDを持ち、`job_checkpoints`が進捗の確定点を保持します。入力・出力解析版や再試行元ジョブへの参照も持ちます。

解析結果の保存時に`post_analysis_intents`を同じトランザクションで記録します。元の`analysis_id`は一意で、補完後の現在版と補完・照合のジョブIDを別に保持し、再起動後も同じ意図を確認します。

## 登録した声と用語

```mermaid
erDiagram
    voice_people ||..o{ voice_samples : "person_id"
    voice_people |o..o{ voice_matches : "person_id"
    speakers ||..o| voice_matches : "speaker_id"
    meetings ||..o{ voice_matches : "meeting_id"
    meetings |o..o| vocabulary_scopes : "meeting_id"
    vocabulary_scopes ||..o{ vocabulary_entries : "scope_id"

    voice_people {
        TEXT id PK
        TEXT name
    }
    voice_samples {
        TEXT id PK
        TEXT person_id FK
        BLOB vector
        TEXT source_meeting_id
        TEXT model_hash
    }
    speakers {
        TEXT id PK
        TEXT analysis_revision_id FK
    }
    voice_matches {
        TEXT id PK
        TEXT meeting_id FK
        TEXT analysis_revision_id FK
        TEXT speaker_id FK
        TEXT person_id FK
        TEXT name_snapshot
        TEXT state
    }
    meetings {
        TEXT id PK
    }
    vocabulary_scopes {
        TEXT id PK
        TEXT meeting_id FK,UK
    }
    vocabulary_entries {
        TEXT id PK
        TEXT scope_id FK
        TEXT term
        TEXT reading
        TEXT note
    }
```

人物と登録特徴量は会議内の話者から独立しています。`voice_samples.source_meeting_id`等は由来の値として保持し、会議への外部キーにはしません。元音声を削除しても登録特徴量と独立保存した登録音声は残ります。`voice_sample_audio.id`は`voice_samples.id`への外部キーで、保存パス・容量・SHA-256・フレーム数・保存状態を管理します。登録区間の削除で音声管理行も削除し、ファイル削除は端末内の`voice_audio_operations`に記録して回収します。人物削除後は照合記録の`person_id`をNULLにし、名前のスナップショットを保持します。

用語のスコープは`global`と会議別です。共通スコープだけは`meeting_id`を持たず、各会議には最大1件のスコープを作ります。実行時に固定する辞書内容は別の`vocabulary_snapshots`に保持します。

モデルパッケージ・端末設定・復元・削除の台帳など、図で省略した領域は[モデル仕様](models.md)・[保存仕様](storage.md)とマイグレーションを参照してください。

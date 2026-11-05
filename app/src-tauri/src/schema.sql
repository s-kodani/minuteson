-- Canonical schema for a new MinutesOn database.

CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);

CREATE TABLE analysis_flows (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id),
    stage TEXT NOT NULL CHECK(stage IN ('pending','configuration','transcription','diarization','attention','review')),
    plan_json TEXT CHECK(plan_json IS NULL OR (json_valid(plan_json) AND length(CAST(plan_json AS BLOB)) <= 65536)),
    plan_hash TEXT CHECK(plan_hash IS NULL OR length(plan_hash)=64),
    transcription_job_id TEXT REFERENCES processing_jobs(id),
    diarization_job_id TEXT REFERENCES processing_jobs(id),
    error_message TEXT,
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK(lock_version>=0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((plan_json IS NULL) = (plan_hash IS NULL)),
    CHECK(stage NOT IN ('transcription','diarization','review') OR plan_json IS NOT NULL),
    CHECK(stage NOT IN ('diarization','review') OR transcription_job_id IS NOT NULL),
    CHECK(stage != 'review' OR diarization_job_id IS NOT NULL)
);

CREATE TABLE analysis_input_snapshots (
    analysis_id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    source_analysis_id TEXT NOT NULL,
    source_content_hash TEXT NOT NULL CHECK (length(source_content_hash) = 64),
    source_draft_json TEXT CHECK (source_draft_json IS NULL OR json_valid(source_draft_json)),
    working_content_hash TEXT NOT NULL CHECK (length(working_content_hash) = 64),
    meeting_title TEXT NOT NULL,
    meeting_created_at INTEGER NOT NULL,
    meeting_started_at_utc_ms INTEGER,
    FOREIGN KEY (meeting_id, analysis_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, source_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);

CREATE TABLE analysis_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    parent_revision_id TEXT,
    revision_no INTEGER NOT NULL CHECK (revision_no > 0),
    parent_content_hash TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('engine', 'manual_snapshot')),
    publication_state TEXT NOT NULL CHECK (publication_state IN ('building', 'candidate', 'committed', 'internal_snapshot')),
    audio_manifest_hash TEXT NOT NULL CHECK (length(audio_manifest_hash) = 64),
    profile_snapshot_json TEXT NOT NULL CHECK (json_valid(profile_snapshot_json)),
    config_json TEXT NOT NULL CHECK (json_valid(config_json)),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    created_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id),
    UNIQUE (meeting_id, revision_no),
    FOREIGN KEY (meeting_id, parent_revision_id) REFERENCES analysis_revisions(meeting_id, id)
);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK (lock_version > 0),
    updated_at INTEGER NOT NULL
);

CREATE TABLE audio_chunks (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    file_asset_id TEXT NOT NULL UNIQUE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    frame_count INTEGER NOT NULL CHECK (frame_count > 0),
    sample_rate INTEGER NOT NULL CHECK (sample_rate > 0),
    channels INTEGER NOT NULL CHECK (channels > 0),
    state TEXT NOT NULL CHECK (state IN ('ready', 'missing', 'corrupt')),
    created_at INTEGER NOT NULL,
    UNIQUE (track_id, id),
    UNIQUE (track_id, sequence),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (meeting_id, file_asset_id) REFERENCES file_assets(meeting_id, id)
);

CREATE TABLE audio_gaps (
    id TEXT PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL REFERENCES audio_tracks(id) ON DELETE RESTRICT,
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    reason_code TEXT NOT NULL,
    recovered INTEGER NOT NULL DEFAULT 0 CHECK (recovered IN (0, 1))
);

CREATE TABLE audio_import_commits (
    import_id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE,
    source_size INTEGER NOT NULL CHECK(source_size > 0),
    source_hash TEXT NOT NULL CHECK(length(source_hash)=64),
    info_json TEXT NOT NULL CHECK(json_valid(info_json) AND length(CAST(info_json AS BLOB))<=8192),
    manifest_asset_id TEXT REFERENCES file_assets(id),
    manifest_hash TEXT CHECK(manifest_hash IS NULL OR length(manifest_hash)=64),
    error_message TEXT,
    CHECK((manifest_asset_id IS NULL)=(manifest_hash IS NULL)),
    FOREIGN KEY(meeting_id,import_id) REFERENCES audio_imports(meeting_id,id)
);

CREATE TABLE audio_import_options (
    import_id TEXT PRIMARY KEY REFERENCES audio_import_commits(import_id),
    options_json TEXT NOT NULL CHECK(json_valid(options_json) AND length(CAST(options_json AS BLOB))<=1048576),
    options_hash TEXT NOT NULL CHECK(length(options_hash)=64)
);

CREATE TABLE audio_imports (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('copying', 'converting', 'ready', 'failed')),
    original_file_name TEXT NOT NULL,
    detected_format TEXT,
    duration_us INTEGER CHECK (duration_us IS NULL OR duration_us >= 0),
    original_asset_id TEXT,
    converted_asset_id TEXT,
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id),
    FOREIGN KEY (meeting_id, original_asset_id) REFERENCES file_assets(meeting_id, id),
    FOREIGN KEY (meeting_id, converted_asset_id) REFERENCES file_assets(meeting_id, id)
);

CREATE TABLE audio_tracks (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    session_id TEXT,
    import_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('mic', 'system', 'imported', 'processed')),
    format TEXT NOT NULL,
    sample_rate INTEGER NOT NULL CHECK (sample_rate > 0),
    channels INTEGER NOT NULL CHECK (channels > 0),
    state TEXT NOT NULL CHECK (state IN ('writing', 'ready', 'interrupted', 'failed')),
    device_label TEXT,
    processing_config_json TEXT CHECK (processing_config_json IS NULL OR json_valid(processing_config_json)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK ((session_id IS NOT NULL) != (import_id IS NOT NULL)),
    UNIQUE (meeting_id, id),
    FOREIGN KEY (meeting_id, session_id) REFERENCES recording_sessions(meeting_id, id),
    FOREIGN KEY (meeting_id, import_id) REFERENCES audio_imports(meeting_id, id)
);

CREATE TABLE diarization_manual_assignments (
    analysis_id TEXT NOT NULL REFERENCES analysis_input_snapshots(analysis_id),
    utterance_id TEXT NOT NULL,
    PRIMARY KEY (analysis_id, utterance_id),
    FOREIGN KEY (analysis_id, utterance_id) REFERENCES utterances(analysis_revision_id, id)
);

CREATE TABLE export_records (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
    exported_content_hash TEXT NOT NULL CHECK(length(exported_content_hash)=64),
    output_content_hash TEXT NOT NULL CHECK(length(output_content_hash)=64),
    output_size_bytes INTEGER NOT NULL CHECK(output_size_bytes >= 0),
    source_draft_id TEXT,
    source_draft_lock_version INTEGER,
    format TEXT NOT NULL CHECK(format='markdown'),
    destination_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('writing','succeeded','failed','interrupted')),
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    exported_at INTEGER,
    CHECK((source_draft_id IS NULL) = (source_draft_lock_version IS NULL)),
    CHECK(source_draft_lock_version IS NULL OR source_draft_lock_version >= 0),
    CHECK((state='succeeded') = (exported_at IS NOT NULL))
);

CREATE TABLE file_assets (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    storage_scope TEXT NOT NULL CHECK (storage_scope = 'meeting'),
    relative_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    hash_algorithm TEXT NOT NULL CHECK (hash_algorithm = 'sha256'),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    state TEXT NOT NULL CHECK (state IN ('ready', 'missing', 'corrupt', 'delete_pending')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (storage_scope, relative_path)
);

CREATE TABLE job_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES processing_jobs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    completed_units INTEGER NOT NULL CHECK (completed_units >= 0),
    checkpoint_json TEXT NOT NULL CHECK (json_valid(checkpoint_json)),
    checkpoint_hash TEXT NOT NULL CHECK (length(checkpoint_hash) = 64),
    created_at INTEGER NOT NULL,
    UNIQUE (job_id, sequence)
);

CREATE TABLE maintenance_operations (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT REFERENCES meetings(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('restore_addition','delete_meeting','delete_assets','move_storage','prune_revisions')),
    state TEXT NOT NULL CHECK(state IN ('prepared','installing','cleanup','recovered','completed','attention')),
    restore_batch_id TEXT UNIQUE REFERENCES restore_batches(batch_id),
    target_manifest_json TEXT NOT NULL CHECK(json_valid(target_manifest_json) AND length(CAST(target_manifest_json AS BLOB))<=65536),
    target_hash TEXT NOT NULL CHECK(length(target_hash)=64),
    consent_at INTEGER,
    progress_json TEXT NOT NULL CHECK(json_valid(progress_json) AND length(CAST(progress_json AS BLOB))<=4096),
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((kind='restore_addition') = (restore_batch_id IS NOT NULL))
);

CREATE TABLE maintenance_paths (
    operation_id TEXT NOT NULL REFERENCES maintenance_operations(id),
    relative_path TEXT NOT NULL CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 256 AND instr(relative_path,char(0))=0),
    kind TEXT NOT NULL CHECK(kind IN ('file','directory','shared_directory')),
    size_bytes INTEGER CHECK(size_bytes IS NULL OR size_bytes>=0),
    content_hash TEXT CHECK(content_hash IS NULL OR length(content_hash)=64),
    identity_json TEXT CHECK(identity_json IS NULL OR (json_valid(identity_json) AND length(CAST(identity_json AS BLOB))<=1024)),
    identity_hash TEXT CHECK(identity_hash IS NULL OR length(identity_hash)=64),
    PRIMARY KEY(operation_id,relative_path),
    CHECK((kind='file') = (size_bytes IS NOT NULL)),
    CHECK((kind='file') = (content_hash IS NOT NULL)),
    CHECK((identity_json IS NULL) = (identity_hash IS NULL))
);

CREATE TABLE meeting_audio_commits (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id),
    track_id TEXT NOT NULL REFERENCES audio_tracks(id),
    manifest_asset_id TEXT NOT NULL REFERENCES file_assets(id),
    manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
    frames INTEGER NOT NULL CHECK(frames>0),
    source_json TEXT NOT NULL CHECK(json_valid(source_json)),
    cleanup_json TEXT NOT NULL CHECK(json_valid(cleanup_json)),
    phase TEXT NOT NULL CHECK(phase IN ('committed','cleaned')),
    created_at INTEGER NOT NULL
);

CREATE TABLE meetings (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    started_at_utc_ms INTEGER,
    duration_us INTEGER NOT NULL DEFAULT 0 CHECK (duration_us >= 0),
    lifecycle TEXT NOT NULL DEFAULT 'draft' CHECK (lifecycle IN ('draft', 'recording', 'recorded', 'imported', 'archived')),
    next_analysis_no INTEGER NOT NULL DEFAULT 1 CHECK (next_analysis_no > 0),
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK (lock_version >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    current_analysis_id TEXT REFERENCES analysis_revisions(id),
    source_type TEXT CHECK (source_type IN ('recording', 'audio_import'))
);

CREATE TABLE minutes_markdown (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    markdown TEXT NOT NULL CHECK (length(CAST(markdown AS BLOB)) <= 2097152),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK (lock_version BETWEEN 1 AND 9007199254740991),
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
    base_lock_version INTEGER NOT NULL CHECK (base_lock_version BETWEEN 0 AND 9007199254740990),
    base_content_hash TEXT CHECK (base_content_hash IS NULL OR length(base_content_hash) = 64),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK (lock_version = base_lock_version + 1),
    CHECK ((base_lock_version = 0) = (base_content_hash IS NULL))
);

CREATE TABLE model_compatibility_checks (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    review_hash TEXT NOT NULL CHECK(length(review_hash)=64),
    adapter_revision INTEGER NOT NULL CHECK(adapter_revision=1),
    state TEXT NOT NULL CHECK(state IN ('preparing','succeeded','failed','cancelled','interrupted')),
    report_json TEXT CHECK(report_json IS NULL OR (json_valid(report_json) AND length(CAST(report_json AS BLOB))<=65536)),
    report_hash TEXT CHECK(report_hash IS NULL OR length(report_hash)=64),
    error_message TEXT CHECK(error_message IS NULL OR length(CAST(error_message AS BLOB))<=4096),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    finished_at INTEGER CHECK(finished_at IS NULL OR finished_at>=created_at),
    CHECK((state='preparing' AND finished_at IS NULL AND report_json IS NULL AND report_hash IS NULL AND error_message IS NULL)
       OR (state='succeeded' AND finished_at IS NOT NULL AND report_json IS NOT NULL AND report_hash IS NOT NULL AND error_message IS NULL)
       OR (state IN ('failed','cancelled','interrupted') AND finished_at IS NOT NULL AND report_json IS NULL AND report_hash IS NULL AND error_message IS NOT NULL))
);

CREATE TABLE model_files (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    relative_path TEXT NOT NULL,
    role TEXT NOT NULL,
    expected_size INTEGER CHECK(expected_size IS NULL OR expected_size>=0),
    actual_size INTEGER NOT NULL CHECK(actual_size>=0),
    expected_hash TEXT CHECK(expected_hash IS NULL OR length(expected_hash)=64),
    actual_hash TEXT NOT NULL CHECK(length(actual_hash)=64),
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible')),
    UNIQUE(package_id,relative_path),
    CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 4096 AND instr(relative_path,char(0))=0
      AND substr(relative_path,1,1)!='/' AND substr(relative_path,-1,1)!='/' AND instr(relative_path,char(92))=0
      AND instr('/'||relative_path||'/','/../')=0 AND instr('/'||relative_path||'/','/./')=0
      AND instr(relative_path,'//')=0 AND relative_path NOT GLOB '[A-Za-z]:*')
);

CREATE TABLE model_packages (
    id TEXT PRIMARY KEY NOT NULL,
    engine TEXT NOT NULL,
    model_name TEXT NOT NULL,
    model_version TEXT NOT NULL,
    quantization TEXT,
    source_type TEXT NOT NULL CHECK(source_type IN ('download','manual_import')),
    manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json) AND length(CAST(manifest_json AS BLOB))<=65536),
    manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
    license_ref TEXT,
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible')),
    UNIQUE(source_type,manifest_hash)
);

CREATE TABLE model_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK(name IN ('standard','high_accuracy')),
    profile_version INTEGER NOT NULL CHECK(profile_version>0),
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible'))
);

CREATE TABLE post_analysis_intents (
 id TEXT PRIMARY KEY,
 meeting_id TEXT NOT NULL REFERENCES meetings(id),
 analysis_id TEXT NOT NULL UNIQUE REFERENCES analysis_revisions(id),
 current_analysis_id TEXT NOT NULL REFERENCES analysis_revisions(id),
 working_hash TEXT NOT NULL CHECK(length(working_hash)=64),
 spec_json TEXT NOT NULL CHECK(json_valid(spec_json) AND length(CAST(spec_json AS BLOB))<=1048576),
 spec_hash TEXT NOT NULL CHECK(length(spec_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('completion_pending','completion_enqueued','matching_pending','matching_enqueued','blocked','completed','stale','restored')),
 completion_operation_id TEXT NOT NULL UNIQUE,
 matching_operation_id TEXT NOT NULL UNIQUE,
 completion_job_id TEXT REFERENCES processing_jobs(id),
 matching_job_id TEXT REFERENCES processing_jobs(id),
 error_message TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);

CREATE TABLE "processing_jobs" (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('transcription', 'diarization', 'minutes_generation', 'voice_registration', 'voice_matching', 'speaker_completion')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'pausing', 'paused', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    operation_id TEXT NOT NULL UNIQUE,
    retry_of_job_id TEXT REFERENCES processing_jobs(id) ON DELETE RESTRICT,
    input_analysis_id TEXT,
    input_hash TEXT NOT NULL CHECK (length(input_hash) = 64),
    profile_snapshot_json TEXT NOT NULL CHECK (json_valid(profile_snapshot_json)),
    parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)),
    spec_hash TEXT NOT NULL CHECK (length(spec_hash) = 64),
    completed_units INTEGER NOT NULL DEFAULT 0 CHECK (completed_units >= 0),
    total_units INTEGER CHECK (total_units IS NULL OR total_units >= 0),
    error_code TEXT,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    output_analysis_id TEXT REFERENCES analysis_revisions(id) ON DELETE RESTRICT,
    pause_reason TEXT CHECK (pause_reason IN ('user', 'recording', 'exit')),
    CHECK (total_units IS NULL OR completed_units <= total_units),
    FOREIGN KEY (meeting_id, input_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);

CREATE TABLE profile_components (
    profile_id TEXT NOT NULL REFERENCES model_profiles(id),
    role TEXT NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    settings_json TEXT NOT NULL CHECK(json_valid(settings_json) AND length(CAST(settings_json AS BLOB))<=65536),
    PRIMARY KEY(profile_id,role)
);

CREATE TABLE recording_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'recording', 'stopping', 'saved', 'interrupted', 'failed')),
    capture_mode TEXT NOT NULL CHECK (capture_mode IN ('system', 'application')),
    capture_config_json TEXT NOT NULL CHECK (json_valid(capture_config_json)),
    start_operation_id TEXT NOT NULL UNIQUE,
    stop_operation_id TEXT UNIQUE,
    started_at_utc_ms INTEGER,
    ended_at_utc_ms INTEGER,
    duration_us INTEGER NOT NULL DEFAULT 0 CHECK (duration_us >= 0),
    audio_manifest_hash TEXT CHECK (audio_manifest_hash IS NULL OR length(audio_manifest_hash) = 64),
    last_committed_us INTEGER NOT NULL DEFAULT 0 CHECK (last_committed_us >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id)
);

CREATE TABLE restore_batches (
    batch_id TEXT PRIMARY KEY,
    archive_id TEXT NOT NULL,
    archive_fingerprint TEXT NOT NULL CHECK(length(archive_fingerprint)=64),
    mapping_hash TEXT NOT NULL CHECK(length(mapping_hash)=64),
    mapping_count INTEGER NOT NULL CHECK(mapping_count>=0),
    state TEXT NOT NULL CHECK(state IN ('prepared','published')),
    created_at INTEGER NOT NULL,
    published_at INTEGER,
    CHECK((state='published') = (published_at IS NOT NULL))
);

CREATE TABLE restore_identity_maps (
    batch_id TEXT NOT NULL REFERENCES restore_batches(batch_id),
    domain TEXT NOT NULL,
    source_scope TEXT NOT NULL,
    source_key TEXT NOT NULL,
    target_key TEXT NOT NULL,
    PRIMARY KEY(batch_id,domain,source_scope,source_key)
);

CREATE TABLE restore_provenance (
    id TEXT PRIMARY KEY,
    meeting_id TEXT REFERENCES meetings(id),
    domain TEXT NOT NULL CHECK(length(domain) BETWEEN 1 AND 64),
    entity_key TEXT NOT NULL CHECK(length(CAST(entity_key AS BLOB)) BETWEEN 1 AND 256 AND instr(entity_key,char(0))=0),
    source_archive TEXT NOT NULL,
    source_scope TEXT NOT NULL CHECK(length(CAST(source_scope AS BLOB))<=256 AND instr(source_scope,char(0))=0),
    source_key TEXT NOT NULL CHECK(length(CAST(source_key AS BLOB)) BETWEEN 1 AND 256 AND instr(source_key,char(0))=0),
    restore_batch TEXT NOT NULL,
    restored_at INTEGER NOT NULL,
    UNIQUE(restore_batch,domain,source_scope,source_key,entity_key)
);

CREATE TABLE speaker_segments (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    speaker_id TEXT,
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    overlap_group_id TEXT,
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    FOREIGN KEY (meeting_id, analysis_revision_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (analysis_revision_id, speaker_id) REFERENCES speakers(analysis_revision_id, id)
);

CREATE TABLE speakers (
    id TEXT PRIMARY KEY NOT NULL,
    analysis_revision_id TEXT NOT NULL REFERENCES analysis_revisions(id) ON DELETE RESTRICT,
    lineage_id TEXT NOT NULL,
    label TEXT NOT NULL,
    display_name TEXT,
    input_scope TEXT NOT NULL,
    identification_state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (analysis_revision_id, id),
    UNIQUE (analysis_revision_id, lineage_id)
);

CREATE TABLE track_clock_segments (
    id TEXT PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL,
    chunk_id TEXT NOT NULL,
    source_frame_start INTEGER NOT NULL CHECK (source_frame_start >= 0),
    source_frame_end INTEGER NOT NULL CHECK (source_frame_end > source_frame_start),
    timeline_start_us INTEGER NOT NULL CHECK (timeline_start_us >= 0),
    timeline_end_us INTEGER NOT NULL CHECK (timeline_end_us > timeline_start_us),
    mapping_json TEXT NOT NULL CHECK (json_valid(mapping_json)),
    FOREIGN KEY (track_id, chunk_id) REFERENCES audio_chunks(track_id, id)
);

CREATE TABLE track_inputs (
    meeting_id TEXT NOT NULL,
    derived_track_id TEXT NOT NULL,
    input_track_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (derived_track_id, input_track_id, role),
    CHECK (derived_track_id != input_track_id),
    FOREIGN KEY (meeting_id, derived_track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (meeting_id, input_track_id) REFERENCES audio_tracks(meeting_id, id)
);

CREATE TABLE utterance_speakers (
    utterance_id TEXT NOT NULL,
    speaker_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('primary', 'overlap')),
    assignment_source TEXT NOT NULL CHECK (assignment_source IN ('inferred', 'manual')),
    PRIMARY KEY (utterance_id, speaker_id),
    FOREIGN KEY (analysis_revision_id, utterance_id) REFERENCES utterances(analysis_revision_id, id),
    FOREIGN KEY (analysis_revision_id, speaker_id) REFERENCES speakers(analysis_revision_id, id)
);

CREATE TABLE utterances (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    lineage_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    recognized_text TEXT NOT NULL,
    effective_text TEXT NOT NULL,
    attribution_state TEXT NOT NULL CHECK (attribution_state IN ('assigned', 'overlap', 'unknown')),
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    UNIQUE (analysis_revision_id, id),
    UNIQUE (analysis_revision_id, lineage_id),
    UNIQUE (analysis_revision_id, sequence),
    FOREIGN KEY (meeting_id, analysis_revision_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id)
);

CREATE TABLE vocabulary_entries (
    id TEXT PRIMARY KEY NOT NULL,
    scope_id TEXT NOT NULL REFERENCES vocabulary_scopes(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK(position >= 0),
    term TEXT NOT NULL CHECK(length(term) BETWEEN 1 AND 200),
    reading TEXT CHECK(reading IS NULL OR length(reading) BETWEEN 1 AND 200),
    note TEXT CHECK(note IS NULL OR length(note) BETWEEN 1 AND 1000),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    lock_version INTEGER NOT NULL CHECK(lock_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(scope_id, term),
    UNIQUE(scope_id, position)
);

CREATE TABLE vocabulary_scopes (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK(lock_version >= 0),
    updated_at INTEGER NOT NULL,
    CHECK ((id = 'global' AND meeting_id IS NULL) OR (meeting_id IS NOT NULL AND id = 'meeting:' || meeting_id))
);

CREATE TABLE vocabulary_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    job_id TEXT NOT NULL UNIQUE,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    created_at INTEGER NOT NULL,
    FOREIGN KEY(meeting_id, job_id) REFERENCES processing_jobs(meeting_id, id)
);

CREATE TABLE voice_audio_operations (
    id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    temporary_path TEXT,
    identity_json TEXT,
    content_hash TEXT,
    size_bytes INTEGER,
    kind TEXT NOT NULL CHECK(kind IN ('install','delete'))
);

CREATE TABLE voice_matches (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id),
    analysis_revision_id TEXT NOT NULL,
    speaker_id TEXT NOT NULL,
    person_id TEXT REFERENCES voice_people(id) ON DELETE SET NULL,
    name_snapshot TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('candidate','automatic','confirmed','dismissed')),
    similarity REAL CHECK(similarity IS NULL OR similarity BETWEEN -1 AND 1),
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    policy_version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(analysis_revision_id,speaker_id),
    FOREIGN KEY(analysis_revision_id,speaker_id) REFERENCES speakers(analysis_revision_id,id),
    FOREIGN KEY(meeting_id,analysis_revision_id) REFERENCES analysis_revisions(meeting_id,id)
);

CREATE TABLE voice_people (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 200),
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    created_at INTEGER NOT NULL
);

CREATE TABLE voice_policies (
    id TEXT PRIMARY KEY NOT NULL,
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    extractor_version TEXT NOT NULL,
    threshold REAL NOT NULL CHECK(threshold BETWEEN -1 AND 1),
    margin REAL NOT NULL CHECK(margin > 0 AND margin <= 2),
    evaluation_hash TEXT NOT NULL CHECK(length(evaluation_hash)=64),
    created_at INTEGER NOT NULL
);

CREATE TABLE voice_sample_audio (
    id TEXT PRIMARY KEY NOT NULL REFERENCES voice_samples(id) ON DELETE CASCADE,
    relative_path TEXT,
    size_bytes INTEGER,
    content_hash TEXT,
    frame_count INTEGER,
    state TEXT NOT NULL CHECK(state IN ('ready','unavailable')),
    CHECK(state='ready' OR (relative_path IS NULL AND size_bytes IS NULL AND content_hash IS NULL AND frame_count IS NULL)),
    CHECK(state!='ready' OR (relative_path IS NOT NULL AND size_bytes IS NOT NULL AND size_bytes>=44 AND content_hash IS NOT NULL AND length(content_hash)=64 AND frame_count IS NOT NULL AND frame_count>0))
);

CREATE TABLE voice_samples (
    id TEXT PRIMARY KEY NOT NULL,
    person_id TEXT NOT NULL REFERENCES voice_people(id) ON DELETE CASCADE,
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    extractor_version TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK(dimension BETWEEN 1 AND 4096),
    vector BLOB NOT NULL CHECK(length(vector)=dimension*4),
    source_meeting_id TEXT,
    source_analysis_id TEXT,
    source_track_id TEXT,
    start_us INTEGER NOT NULL CHECK(start_us>=0),
    end_us INTEGER NOT NULL CHECK(end_us>start_us),
    created_at INTEGER NOT NULL
);

CREATE TABLE working_drafts (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    target_kind TEXT NOT NULL CHECK (target_kind = 'transcript'),
    base_analysis_id TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK (lock_version >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (meeting_id, base_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);

CREATE INDEX audio_chunks_time ON audio_chunks(track_id, start_us);

CREATE UNIQUE INDEX audio_import_single_active ON audio_imports((1)) WHERE state IN ('copying','converting');

CREATE INDEX export_records_meeting ON export_records(meeting_id,created_at DESC,id);

CREATE INDEX file_assets_meeting ON file_assets(meeting_id, state);

CREATE UNIQUE INDEX file_assets_meeting_id_id ON file_assets(meeting_id, id);

CREATE UNIQUE INDEX job_one_unfinished_retry ON processing_jobs(retry_of_job_id)
WHERE retry_of_job_id IS NOT NULL AND state IN ('queued', 'running', 'pausing', 'paused', 'interrupted');

CREATE INDEX maintenance_operations_state ON maintenance_operations(kind,state);

CREATE INDEX meetings_updated_at ON meetings(updated_at DESC, id DESC);

CREATE INDEX model_compatibility_package ON model_compatibility_checks(package_id);

CREATE INDEX model_files_fingerprint ON model_files(actual_hash,actual_size,role);

CREATE UNIQUE INDEX model_profile_version ON model_profiles(name,profile_version);

CREATE UNIQUE INDEX one_model_compatibility_check ON model_compatibility_checks((1)) WHERE state='preparing';

CREATE INDEX processing_jobs_meeting_created ON processing_jobs(meeting_id, created_at);

CREATE UNIQUE INDEX processing_jobs_meeting_identity ON processing_jobs(meeting_id, id);

CREATE UNIQUE INDEX processing_jobs_one_active ON processing_jobs((1)) WHERE state IN ('running', 'pausing');

CREATE INDEX processing_jobs_state_created ON processing_jobs(state, created_at);

CREATE UNIQUE INDEX recording_sessions_one_active
ON recording_sessions((1))
WHERE state IN ('preparing', 'recording', 'stopping');

CREATE INDEX restore_provenance_meeting ON restore_provenance(meeting_id,domain,entity_key);

CREATE INDEX speaker_segments_time ON speaker_segments(analysis_revision_id, start_us);

CREATE INDEX utterances_time ON utterances(analysis_revision_id, start_us);

CREATE INDEX voice_samples_person ON voice_samples(person_id,model_hash);

CREATE TRIGGER analysis_committed_immutable BEFORE UPDATE ON analysis_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis is immutable'); END;

CREATE TRIGGER analysis_committed_no_delete BEFORE DELETE ON analysis_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis cannot be deleted'); END;

CREATE TRIGGER analysis_flow_job_scope BEFORE UPDATE ON analysis_flows
WHEN (NEW.transcription_job_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM processing_jobs WHERE id=NEW.transcription_job_id AND meeting_id=NEW.meeting_id AND kind='transcription'))
 OR (NEW.diarization_job_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM processing_jobs WHERE id=NEW.diarization_job_id AND meeting_id=NEW.meeting_id AND kind='diarization'))
BEGIN SELECT RAISE(ABORT, 'analysis flow job has wrong kind or meeting'); END;

CREATE TRIGGER analysis_flow_plan_fixed BEFORE UPDATE ON analysis_flows
WHEN NEW.id != OLD.id OR NEW.meeting_id != OLD.meeting_id
 OR (OLD.plan_json IS NOT NULL AND (NEW.plan_json IS NOT OLD.plan_json OR NEW.plan_hash IS NOT OLD.plan_hash))
BEGIN SELECT RAISE(ABORT, 'analysis flow identity and fixed plan are immutable'); END;

CREATE TRIGGER analysis_internal_fixed_delete BEFORE DELETE ON analysis_revisions
WHEN OLD.publication_state = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis cannot be deleted'); END;

CREATE TRIGGER analysis_internal_fixed_update BEFORE UPDATE ON analysis_revisions
WHEN OLD.publication_state = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis is immutable'); END;

CREATE TRIGGER analysis_parent_committed BEFORE INSERT ON analysis_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM analysis_revisions p WHERE p.id=NEW.parent_revision_id
 AND p.meeting_id=NEW.meeting_id AND (p.publication_state='committed' OR (
  p.publication_state='candidate' AND EXISTS (
   SELECT 1 FROM analysis_flows f JOIN processing_jobs t ON t.id=f.transcription_job_id
   WHERE f.meeting_id=NEW.meeting_id AND t.meeting_id=f.meeting_id
   AND t.kind='transcription' AND t.state='succeeded' AND t.output_analysis_id=p.id
   AND t.input_hash=p.audio_manifest_hash)
  AND ((NEW.origin='manual_snapshot' AND NEW.publication_state='building'
        AND json_extract(NEW.config_json,'$.inputSnapshot.sourceAnalysisId')=p.id)
    OR (NEW.origin='engine' AND NEW.publication_state='candidate' AND EXISTS (
      SELECT 1 FROM processing_jobs d JOIN analysis_input_snapshots s ON s.analysis_id=d.input_analysis_id
      WHERE d.id=json_extract(NEW.config_json,'$.jobId') AND d.meeting_id=NEW.meeting_id
      AND d.kind='diarization' AND d.state='running' AND s.source_analysis_id=p.id
      AND d.input_analysis_id=json_extract(NEW.config_json,'$.inputAnalysisId')
      AND d.input_hash=json_extract(NEW.config_json,'$.inputContentHash')))))))
BEGIN SELECT RAISE(ABORT, 'analysis parent is not committed or a verified flow source'); END;

CREATE TRIGGER analysis_parent_hash BEFORE INSERT ON analysis_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND
     (NEW.parent_content_hash IS NULL OR NEW.parent_content_hash !=
      (SELECT content_hash FROM analysis_revisions WHERE id = NEW.parent_revision_id AND meeting_id = NEW.meeting_id))
BEGIN SELECT RAISE(ABORT, 'analysis parent hash mismatch'); END;

CREATE TRIGGER assignments_frozen_delete BEFORE DELETE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER assignments_frozen_insert BEFORE INSERT ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER assignments_frozen_update BEFORE UPDATE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_delete BEFORE DELETE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_insert BEFORE INSERT ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_update BEFORE UPDATE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER audio_import_identity_fixed BEFORE UPDATE ON audio_import_commits
WHEN NEW.import_id!=OLD.import_id OR NEW.meeting_id!=OLD.meeting_id
 OR NEW.operation_id!=OLD.operation_id OR NEW.source_size!=OLD.source_size
 OR NEW.source_hash!=OLD.source_hash OR NEW.info_json!=OLD.info_json
 OR (OLD.manifest_asset_id IS NOT NULL AND (NEW.manifest_asset_id IS NOT OLD.manifest_asset_id OR NEW.manifest_hash IS NOT OLD.manifest_hash))
BEGIN SELECT RAISE(ABORT,'import provenance is immutable'); END;

CREATE TRIGGER audio_import_options_fixed BEFORE UPDATE ON audio_import_options
BEGIN SELECT RAISE(ABORT,'initial import options are immutable'); END;

CREATE TRIGGER audio_import_ready_assets BEFORE UPDATE OF state ON audio_imports
WHEN NEW.state='ready' AND NOT EXISTS (
 SELECT 1 FROM audio_import_commits p
 JOIN file_assets m ON m.id=p.manifest_asset_id AND m.meeting_id=p.meeting_id AND m.state='ready' AND m.content_hash=p.manifest_hash AND m.media_type='application/json'
 JOIN file_assets o ON o.id=NEW.original_asset_id AND o.meeting_id=p.meeting_id AND o.state='ready' AND o.content_hash=p.source_hash AND o.size_bytes=p.source_size
 JOIN file_assets a ON a.id=NEW.converted_asset_id AND a.meeting_id=p.meeting_id AND a.state='ready' AND a.media_type='audio/x-caf'
 WHERE p.import_id=NEW.id AND p.meeting_id=NEW.meeting_id)
BEGIN SELECT RAISE(ABORT,'ready import requires verified scoped assets'); END;

CREATE TRIGGER audio_import_ready_fixed BEFORE UPDATE ON audio_imports
WHEN OLD.state='ready'
BEGIN SELECT RAISE(ABORT,'ready import is immutable'); END;

CREATE TRIGGER audio_import_recording_exclusive BEFORE INSERT ON audio_imports
WHEN EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping'))
BEGIN SELECT RAISE(ABORT,'recording is active'); END;

CREATE TRIGGER chunk_requires_ready_asset BEFORE INSERT ON audio_chunks
WHEN NOT EXISTS (SELECT 1 FROM file_assets WHERE id = NEW.file_asset_id AND meeting_id = NEW.meeting_id AND state = 'ready')
BEGIN SELECT RAISE(ABORT, 'chunk asset is not ready'); END;

CREATE TRIGGER current_job_format BEFORE INSERT ON processing_jobs
WHEN (NEW.kind='transcription' AND json_extract(NEW.parameters_json,'$.schemaVersion') IS NOT 4)
 OR (NEW.kind='diarization' AND json_extract(NEW.parameters_json,'$.schemaVersion') IS NOT 3)
 OR (NEW.kind='minutes_generation' AND json_extract(NEW.parameters_json,'$.generation.schemaVersion') IS NOT 9)
 OR (NEW.kind IN ('voice_registration','voice_matching') AND json_extract(NEW.parameters_json,'$.voice.schemaVersion') IS NOT 4)
 OR (NEW.kind='speaker_completion' AND json_extract(NEW.parameters_json,'$.completion.schemaVersion') IS NOT 2)
BEGIN SELECT RAISE(ABORT, 'unsupported processing job format'); END;

CREATE TRIGGER diarization_manual_delete BEFORE DELETE ON diarization_manual_assignments
BEGIN SELECT RAISE(ABORT, 'diarization assignments cannot be deleted'); END;

CREATE TRIGGER diarization_manual_insert BEFORE INSERT ON diarization_manual_assignments
WHEN EXISTS (SELECT 1 FROM processing_jobs WHERE input_analysis_id = NEW.analysis_id)
  OR NOT EXISTS (SELECT 1 FROM analysis_revisions WHERE id = NEW.analysis_id AND publication_state = 'internal_snapshot')
BEGIN SELECT RAISE(ABORT, 'diarization assignments are not a new fixed input'); END;

CREATE TRIGGER diarization_manual_update BEFORE UPDATE ON diarization_manual_assignments
BEGIN SELECT RAISE(ABORT, 'diarization assignments are immutable'); END;

CREATE TRIGGER draft_base_fixed BEFORE UPDATE OF meeting_id, target_kind, base_analysis_id ON working_drafts
BEGIN SELECT RAISE(ABORT, 'draft base must be rebased by a separate operation'); END;

CREATE TRIGGER import_source_match BEFORE INSERT ON audio_imports
WHEN (SELECT source_type FROM meetings WHERE id = NEW.meeting_id) IS NOT 'audio_import'
BEGIN SELECT RAISE(ABORT, 'meeting is not an import source'); END;

CREATE TRIGGER imported_track_kind BEFORE INSERT ON audio_tracks
WHEN NEW.import_id IS NOT NULL AND NEW.kind NOT IN ('imported','processed')
BEGIN SELECT RAISE(ABORT,'import does not provide microphone or system tracks'); END;

CREATE TRIGGER imported_track_kind_update BEFORE UPDATE OF import_id,kind ON audio_tracks
WHEN NEW.import_id IS NOT NULL AND NEW.kind NOT IN ('imported','processed')
BEGIN SELECT RAISE(ABORT,'import does not provide microphone or system tracks'); END;

CREATE TRIGGER input_snapshot_fixed_delete BEFORE DELETE ON analysis_input_snapshots
BEGIN SELECT RAISE(ABORT, 'input snapshot metadata cannot be deleted'); END;

CREATE TRIGGER input_snapshot_fixed_update BEFORE UPDATE ON analysis_input_snapshots
BEGIN SELECT RAISE(ABORT, 'input snapshot metadata is immutable'); END;

CREATE TRIGGER input_snapshot_valid BEFORE INSERT ON analysis_input_snapshots
WHEN NOT EXISTS (
 SELECT 1 FROM analysis_revisions a JOIN analysis_revisions p
 ON p.id=a.parent_revision_id AND p.meeting_id=a.meeting_id
 WHERE a.id=NEW.analysis_id AND a.meeting_id=NEW.meeting_id
 AND a.publication_state='internal_snapshot' AND a.origin='manual_snapshot'
 AND p.id=NEW.source_analysis_id AND p.content_hash=NEW.source_content_hash
 AND a.parent_content_hash=NEW.source_content_hash
 AND (p.publication_state='committed' OR (p.publication_state='candidate' AND EXISTS (
   SELECT 1 FROM analysis_flows f JOIN processing_jobs t ON t.id=f.transcription_job_id
   WHERE f.meeting_id=NEW.meeting_id AND t.meeting_id=f.meeting_id
   AND t.kind='transcription' AND t.state='succeeded' AND t.output_analysis_id=p.id
   AND t.input_hash=p.audio_manifest_hash))))
BEGIN SELECT RAISE(ABORT, 'invalid input snapshot provenance'); END;

CREATE TRIGGER job_analysis_result_insert BEFORE INSERT ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'analysis result must be finalized atomically'); END;

CREATE TRIGGER job_attempt_fixed BEFORE UPDATE OF attempt ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job retry attempt is immutable'); END;

CREATE TRIGGER job_checkpoint_progress BEFORE INSERT ON job_checkpoints
WHEN NEW.completed_units < (SELECT completed_units FROM processing_jobs WHERE id = NEW.job_id)
  OR NEW.sequence IS NOT (SELECT COALESCE(MAX(sequence), 0) + 1 FROM job_checkpoints WHERE job_id = NEW.job_id)
BEGIN SELECT RAISE(ABORT, 'job checkpoint is stale'); END;

CREATE TRIGGER job_input_committed BEFORE INSERT ON processing_jobs
WHEN NEW.input_analysis_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state IN ('committed', 'internal_snapshot')
    AND content_hash = NEW.input_hash)
BEGIN SELECT RAISE(ABORT, 'job input analysis is not fixed'); END;

CREATE TRIGGER job_parent_retry_no_resume BEFORE UPDATE OF state ON processing_jobs
WHEN NEW.state = 'queued' AND OLD.state IN ('paused', 'interrupted')
  AND EXISTS(SELECT 1 FROM processing_jobs c WHERE c.retry_of_job_id = OLD.id)
BEGIN SELECT RAISE(ABORT, 'retry parent cannot resume'); END;

CREATE TRIGGER job_result_fixed BEFORE UPDATE OF output_analysis_id ON processing_jobs
WHEN OLD.output_analysis_id IS NOT NULL AND NEW.output_analysis_id IS NOT OLD.output_analysis_id
BEGIN SELECT RAISE(ABORT, 'job result cannot change'); END;

CREATE TRIGGER job_result_scope BEFORE UPDATE OF output_analysis_id, state ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL AND (
    NEW.state != 'succeeded' OR NOT (
        (NEW.kind = 'transcription' AND EXISTS (
            SELECT 1 FROM analysis_revisions a WHERE a.id = NEW.output_analysis_id
            AND a.meeting_id = NEW.meeting_id AND a.audio_manifest_hash = NEW.input_hash))
        OR (NEW.kind = 'diarization' AND EXISTS (
            SELECT 1 FROM analysis_revisions a
            JOIN analysis_revisions i ON i.id = NEW.input_analysis_id
            JOIN analysis_input_snapshots s ON s.analysis_id = i.id
            WHERE a.id = NEW.output_analysis_id AND a.meeting_id = NEW.meeting_id
            AND i.meeting_id = NEW.meeting_id AND s.meeting_id = NEW.meeting_id
            AND i.publication_state = 'internal_snapshot' AND i.content_hash = NEW.input_hash
            AND a.audio_manifest_hash = i.audio_manifest_hash
            AND a.parent_revision_id = s.source_analysis_id AND a.parent_content_hash = s.source_content_hash
            AND json_extract(a.config_json,'$.jobId') = NEW.id
            AND json_extract(a.config_json,'$.specHash') = NEW.spec_hash
            AND json_extract(a.config_json,'$.inputAnalysisId') = NEW.input_analysis_id
            AND json_extract(a.config_json,'$.inputContentHash') = NEW.input_hash))
    ))
BEGIN SELECT RAISE(ABORT, 'job result is not a matching analysis'); END;

CREATE TRIGGER job_retry_valid BEFORE INSERT ON processing_jobs
WHEN (NEW.retry_of_job_id IS NULL AND NEW.attempt != 1)
  OR (NEW.retry_of_job_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM processing_jobs p WHERE p.id = NEW.retry_of_job_id
      AND p.state IN ('failed', 'cancelled')
      AND p.meeting_id = NEW.meeting_id AND p.kind = NEW.kind
      AND p.input_analysis_id IS NEW.input_analysis_id AND p.input_hash = NEW.input_hash
      AND p.profile_snapshot_json = NEW.profile_snapshot_json
      AND p.parameters_json = NEW.parameters_json AND p.total_units IS NEW.total_units
      AND NEW.attempt = p.attempt + 1))
BEGIN SELECT RAISE(ABORT, 'retry inputs or source are not fixed'); END;

CREATE TRIGGER job_spec_fixed BEFORE UPDATE OF meeting_id, kind, operation_id, retry_of_job_id,
    input_analysis_id, input_hash, profile_snapshot_json, parameters_json, spec_hash ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job inputs are immutable'); END;

CREATE TRIGGER job_total_units_fixed BEFORE UPDATE OF total_units ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job planned units are immutable'); END;

CREATE TRIGGER maintenance_operation_fixed BEFORE UPDATE ON maintenance_operations
WHEN NEW.id!=OLD.id OR NEW.kind!=OLD.kind
 OR NEW.restore_batch_id IS NOT OLD.restore_batch_id
 OR NEW.target_manifest_json!=OLD.target_manifest_json OR NEW.target_hash!=OLD.target_hash
 OR NEW.consent_at IS NOT OLD.consent_at OR NEW.created_at!=OLD.created_at
 OR (OLD.state='completed' AND NEW.state!='completed')
BEGIN SELECT RAISE(ABORT,'maintenance intent is immutable'); END;

CREATE TRIGGER maintenance_operation_retained BEFORE DELETE ON maintenance_operations
BEGIN SELECT RAISE(ABORT,'maintenance intent must be retained'); END;

CREATE TRIGGER maintenance_path_fixed BEFORE UPDATE ON maintenance_paths
WHEN NEW.operation_id!=OLD.operation_id OR NEW.relative_path!=OLD.relative_path OR NEW.kind!=OLD.kind
 OR NEW.size_bytes IS NOT OLD.size_bytes OR NEW.content_hash IS NOT OLD.content_hash
 OR NOT EXISTS(SELECT 1 FROM maintenance_operations WHERE id=OLD.operation_id AND
   ((state='installing' AND OLD.identity_json IS NULL AND NEW.identity_json IS NOT NULL)
    OR (state='cleanup' AND NEW.identity_json IS NULL)))
BEGIN SELECT RAISE(ABORT,'maintenance target is immutable except owned identity'); END;

CREATE TRIGGER maintenance_path_prepared BEFORE INSERT ON maintenance_paths
WHEN NOT EXISTS(SELECT 1 FROM maintenance_operations WHERE id=NEW.operation_id AND state='prepared')
BEGIN SELECT RAISE(ABORT,'maintenance path requires prepared intent'); END;

CREATE TRIGGER maintenance_path_retained BEFORE DELETE ON maintenance_paths
BEGIN SELECT RAISE(ABORT,'maintenance targets must be retained'); END;

CREATE TRIGGER meeting_source_immutable BEFORE UPDATE OF source_type ON meetings
WHEN OLD.source_type IS NOT NULL AND NEW.source_type IS NOT OLD.source_type
BEGIN SELECT RAISE(ABORT, 'meeting source cannot change'); END;

CREATE TRIGGER meetings_revision_heads_valid BEFORE UPDATE OF current_analysis_id ON meetings
WHEN NEW.current_analysis_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.current_analysis_id
    AND meeting_id = NEW.id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'invalid current analysis'); END;

CREATE TRIGGER model_compatibility_fixed BEFORE UPDATE ON model_compatibility_checks
WHEN OLD.state!='preparing' OR NEW.id!=OLD.id OR NEW.package_id!=OLD.package_id
 OR NEW.review_hash!=OLD.review_hash OR NEW.adapter_revision!=OLD.adapter_revision OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'model trial identity or completed evidence is immutable'); END;

CREATE TRIGGER model_profile_fixed BEFORE UPDATE ON model_profiles
WHEN OLD.state!='preparing' OR NEW.id!=OLD.id OR NEW.name!=OLD.name OR NEW.profile_version!=OLD.profile_version
 OR NEW.state NOT IN ('ready','unavailable')
BEGIN SELECT RAISE(ABORT,'completed model profile is immutable'); END;

CREATE TRIGGER model_profile_no_delete BEFORE DELETE ON model_profiles
BEGIN SELECT RAISE(ABORT,'model profile history is retained'); END;

CREATE TRIGGER post_analysis_intent_fixed BEFORE UPDATE ON post_analysis_intents
WHEN NEW.id!=OLD.id OR NEW.meeting_id!=OLD.meeting_id OR NEW.analysis_id!=OLD.analysis_id
 OR NEW.spec_json!=OLD.spec_json OR NEW.spec_hash!=OLD.spec_hash
 OR NEW.completion_operation_id!=OLD.completion_operation_id OR NEW.matching_operation_id!=OLD.matching_operation_id
BEGIN SELECT RAISE(ABORT,'follow-up intent inputs are immutable'); END;

CREATE TRIGGER profile_component_fixed BEFORE UPDATE ON profile_components
BEGIN SELECT RAISE(ABORT,'model profile components are immutable'); END;

CREATE TRIGGER profile_component_insert BEFORE INSERT ON profile_components
WHEN NOT EXISTS(SELECT 1 FROM model_profiles WHERE id=NEW.profile_id AND state='preparing')
BEGIN SELECT RAISE(ABORT,'profile components require an unsealed profile'); END;

CREATE TRIGGER profile_component_no_delete BEFORE DELETE ON profile_components
BEGIN SELECT RAISE(ABORT,'model profile components are retained'); END;

CREATE TRIGGER recording_audio_import_exclusive BEFORE INSERT ON recording_sessions
WHEN EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting'))
BEGIN SELECT RAISE(ABORT,'audio import is active'); END;

CREATE TRIGGER recording_source_match BEFORE INSERT ON recording_sessions
WHEN (SELECT source_type FROM meetings WHERE id = NEW.meeting_id) IS NOT 'recording'
BEGIN SELECT RAISE(ABORT, 'meeting is not a recording source'); END;

CREATE TRIGGER restore_batch_fixed BEFORE UPDATE ON restore_batches
WHEN NEW.batch_id != OLD.batch_id OR NEW.archive_id != OLD.archive_id
 OR NEW.archive_fingerprint != OLD.archive_fingerprint OR NEW.mapping_hash != OLD.mapping_hash
 OR NEW.mapping_count != OLD.mapping_count OR NEW.created_at != OLD.created_at
 OR OLD.state='published' OR NEW.state!='published'
BEGIN SELECT RAISE(ABORT, 'restore batch is immutable except publication'); END;

CREATE TRIGGER restore_batch_retained BEFORE DELETE ON restore_batches
BEGIN SELECT RAISE(ABORT, 'restore batch must be retained'); END;

CREATE TRIGGER restore_map_fixed BEFORE UPDATE ON restore_identity_maps
BEGIN SELECT RAISE(ABORT, 'restore identity map is immutable'); END;

CREATE TRIGGER restore_map_retained BEFORE DELETE ON restore_identity_maps
BEGIN SELECT RAISE(ABORT, 'restore identity map must be retained'); END;

CREATE TRIGGER restore_map_unpublished BEFORE INSERT ON restore_identity_maps
WHEN NOT EXISTS(SELECT 1 FROM restore_batches WHERE batch_id=NEW.batch_id AND state='prepared')
BEGIN SELECT RAISE(ABORT, 'restore identity map requires prepared batch'); END;

CREATE TRIGGER restore_provenance_fixed BEFORE UPDATE ON restore_provenance
BEGIN SELECT RAISE(ABORT, 'restore provenance is immutable'); END;

CREATE TRIGGER segments_frozen_delete BEFORE DELETE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER segments_frozen_insert BEFORE INSERT ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER segments_frozen_update BEFORE UPDATE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER segments_internal_delete BEFORE DELETE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER segments_internal_insert BEFORE INSERT ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER segments_internal_update BEFORE UPDATE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER speakers_frozen_delete BEFORE DELETE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER speakers_frozen_insert BEFORE INSERT ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER speakers_frozen_update BEFORE UPDATE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER speakers_internal_delete BEFORE DELETE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER speakers_internal_insert BEFORE INSERT ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER speakers_internal_update BEFORE UPDATE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER track_inputs_no_cycle BEFORE INSERT ON track_inputs
WHEN EXISTS (
    WITH RECURSIVE inputs(id) AS (
        SELECT NEW.input_track_id
        UNION
        SELECT track_inputs.input_track_id FROM track_inputs JOIN inputs ON track_inputs.derived_track_id = inputs.id
    ) SELECT 1 FROM inputs WHERE id = NEW.derived_track_id
)
BEGIN SELECT RAISE(ABORT, 'track input cycle'); END;

CREATE TRIGGER track_inputs_no_update BEFORE UPDATE ON track_inputs
BEGIN SELECT RAISE(ABORT, 'replace track inputs by delete and insert'); END;

CREATE TRIGGER transcript_draft_base BEFORE INSERT ON working_drafts
WHEN NEW.target_kind = 'transcript' AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.base_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'transcript draft requires committed analysis'); END;

CREATE TRIGGER utterances_frozen_delete BEFORE DELETE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER utterances_frozen_insert BEFORE INSERT ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER utterances_frozen_update BEFORE UPDATE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_delete BEFORE DELETE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_insert BEFORE INSERT ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_update BEFORE UPDATE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER vocabulary_snapshot_no_update BEFORE UPDATE ON vocabulary_snapshots
BEGIN SELECT RAISE(ABORT, 'vocabulary snapshot is immutable'); END;

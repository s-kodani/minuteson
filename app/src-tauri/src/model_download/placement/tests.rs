use super::*;
use std::cell::Cell;

struct Fixture {
    _temp: tempfile::TempDir,
    storage: crate::storage::Storage,
    staging: PathBuf,
    models: PathBuf,
    receipt: DownloadReceipt,
    package: String,
    recipe: Recipe,
}
impl Fixture {
    fn new() -> Self {
        Self::with_model(b"fake model bytes".to_vec())
    }
    fn with_model(model: Vec<u8>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let files: [&[u8]; 2] = [b"fake engine never executed", &model];
        let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let recipe: Recipe = serde_json::from_value(serde_json::json!({
            "schemaVersion":4,"manifest":{"schemaVersion":1,"engine":"whisper.cpp","engineVersion":"test",
                "modelName":"move test","modelVersion":"1","quantization":null,"licenseRef":"test",
                "target":model_inspection::Target::current().unwrap(),"files":[
                    {"relativePath":"engine.exe","role":"executable","expectedSize":files[0].len(),"expectedHash":hash(files[0])},
                    {"relativePath":"model.bin","role":"asr","expectedSize":files[1].len(),"expectedHash":hash(files[1])}]},
            "sources":[{"relativePath":"engine.exe","url":"https://example.com/engine"},{"relativePath":"model.bin","url":"https://example.com/model"}]
        })).unwrap();
        let staging = root.join("downloads");
        let operation = uuid::Uuid::new_v4().to_string();
        let receipt = super::super::stage(
            &staging,
            &operation,
            &recipe,
            &recipe.hash().unwrap(),
            &|| false,
            &mut |_, _| {},
            &mut |url, _, _, output, _, progress| {
                let bytes = if url.ends_with("engine") {
                    files[0]
                } else {
                    files[1]
                };
                output.write_all(bytes).map_err(|e| e.to_string())?;
                progress(bytes.len() as u64);
                Ok(())
            },
        )
        .unwrap();
        Self {
            storage: crate::storage::Storage::open(root.join("data")).unwrap(),
            staging,
            models: root.join("models"),
            receipt,
            package: uuid::Uuid::new_v4().to_string(),
            recipe,
            _temp: temp,
        }
    }
    fn place(
        &self,
        cancelled: &impl Fn() -> bool,
        verifying: &impl Fn(),
    ) -> Result<PlacementReceipt, String> {
        place(
            &self.storage,
            &self.models,
            &self.staging,
            &self.receipt.operation_id,
            &self.receipt.recipe_hash,
            &self.package,
            &self.receipt.report.review_hash,
            cancelled,
            verifying,
        )
    }
    fn listing(&self) -> serde_json::Value {
        serde_json::to_value(super::super::catalog::catalog(&self.staging, &|| false).unwrap())
            .unwrap()
    }
}

#[test]
#[ignore = "requires MON_PLACEMENT_BENCHMARK_MODEL; copies only into owned temporary fixtures"]
fn placement_benchmark_move_against_previous_copy_path() {
    let model = std::fs::read(std::env::var_os("MON_PLACEMENT_BENCHMARK_MODEL").unwrap()).unwrap();
    let mut copy_ms = Vec::new();
    let mut move_ms = Vec::new();
    for sample in 0..3 {
        let old = Fixture::with_model(model.clone());
        let new = Fixture::with_model(model.clone());
        let copy = || {
            let start = std::time::Instant::now();
            let recovered = super::super::readback(
                &old.staging,
                &old.receipt.operation_id,
                &old.receipt.recipe_hash,
                &|| false,
            )
            .unwrap();
            let receipt = recovered.receipt.as_ref().unwrap();
            let record = PlacementRecord::downloaded(
                &old.package,
                &receipt.report,
                old.recipe.origin(&old.receipt.operation_id).unwrap(),
            )
            .unwrap();
            crate::model_placement::place_reviewed(
                &old.storage,
                &old.models,
                &receipt.directory,
                &receipt.report,
                &record,
                &|| false,
                &|| recovered.verify(),
            )
            .unwrap();
            start.elapsed().as_millis()
        };
        let moved = || {
            let start = std::time::Instant::now();
            new.place(&|| false, &|| {}).unwrap();
            start.elapsed().as_millis()
        };
        if sample % 2 == 0 {
            copy_ms.push(copy());
            move_ms.push(moved());
        } else {
            move_ms.push(moved());
            copy_ms.push(copy());
        }
    }
    eprintln!(
        "placement_benchmark: {}",
        serde_json::json!({"modelBytes":model.len(),"copyMs":copy_ms,"moveMs":move_ms,"cache":"warm, alternating order"})
    );
}
#[test]
fn placement_moves_directory_without_copy_and_records_verified_destination() {
    let f = Fixture::new();
    let original = Directory::open_existing(&f.receipt.directory)
        .unwrap()
        .identity()
        .unwrap();
    let calls = Cell::new(0);
    let result = f.place(&|| false, &|| calls.set(calls.get() + 1)).unwrap();
    assert!(result.placement_recorded && !result.ready);
    assert!(!f.receipt.directory.exists());
    assert_eq!(
        Directory::open_existing(&f.models.join(&f.package))
            .unwrap()
            .identity()
            .unwrap(),
        original
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(f.listing()["entries"][0]["placement"]["state"], "committed");
    assert!(f.place(&|| false, &|| {}).unwrap().placement_recorded);
    assert_eq!(f.storage.list_model_placements().unwrap().len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.models.join(&f.package).join("engine.exe"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
#[test]
fn placement_cancellation_after_move_keeps_destination_and_resumes_same_package() {
    let f = Fixture::new();
    let stopped = Cell::new(false);
    assert!(f.place(&|| stopped.get(), &|| stopped.set(true)).is_err());
    assert!(!f.receipt.directory.exists());
    assert!(f.models.join(&f.package).is_dir());
    assert!(f.storage.list_model_placements().unwrap().is_empty());
    assert_eq!(f.listing()["entries"][0]["placement"]["state"], "pending");
    assert!(f.place(&|| false, &|| {}).unwrap().placement_recorded);
}
#[test]
fn placement_rejects_changed_bytes_and_foreign_destinations_without_overwrite() {
    let f = Fixture::new();
    std::fs::write(f.receipt.directory.join("model.bin"), b"changed").unwrap();
    assert!(f.place(&|| false, &|| {}).is_err());
    assert!(f.storage.list_model_placements().unwrap().is_empty());
    assert_eq!(
        std::fs::read(f.models.join(&f.package).join("model.bin")).unwrap(),
        b"changed"
    );
    let f = Fixture::new();
    let root = Directory::root(&f.models).unwrap();
    root.child(&f.package, true).unwrap();
    assert!(f.place(&|| false, &|| {}).is_err());
    assert!(f.receipt.directory.is_dir());
    assert!(f.storage.list_model_placements().unwrap().is_empty());
}
#[test]
fn placement_copy_fallback_is_restartable_and_keeps_original_staging() {
    // Exercise the same fallback used after an EXDEV result, without requiring
    // a second physical disk on the test machine.
    let f = Fixture::new();
    let operation = Directory::open_existing(&f.staging.join(&f.receipt.operation_id)).unwrap();
    let root = Directory::root(&f.models).unwrap();
    let transfer = Transfer {
        version: 1,
        download_id: f.receipt.operation_id.clone(),
        package_id: f.package.clone(),
        recipe_hash: f.receipt.recipe_hash.clone(),
        review_hash: f.receipt.report.review_hash.clone(),
        source: Directory::open_existing(&f.receipt.directory)
            .unwrap()
            .identity()
            .unwrap(),
        destination_root: root.identity().unwrap(),
    };
    save(&operation, TRANSFER, &transfer).unwrap();
    copy_and_move(&operation, &root, &f.recipe, &transfer, &|| false).unwrap();
    drop(operation);
    drop(root);
    assert!(f.receipt.directory.is_dir());
    assert!(f.place(&|| false, &|| {}).unwrap().placement_recorded);
    assert_eq!(
        std::fs::read(f.models.join(&f.package).join("model.bin")).unwrap(),
        b"fake model bytes"
    );
}
#[test]
fn placement_rejects_rebinding_a_pending_transfer_to_a_new_package() {
    let f = Fixture::new();
    let stopped = Cell::new(false);
    assert!(f.place(&|| stopped.get(), &|| stopped.set(true)).is_err());
    assert!(place(
        &f.storage,
        &f.models,
        &f.staging,
        &f.receipt.operation_id,
        &f.receipt.recipe_hash,
        &uuid::Uuid::new_v4().to_string(),
        &f.receipt.report.review_hash,
        &|| false,
        &|| {}
    )
    .is_err());
    assert!(f.models.join(&f.package).is_dir());
}

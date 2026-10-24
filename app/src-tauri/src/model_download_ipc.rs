//! Main-window boundary. Every filesystem writer/reader holds the shared model
//! preparation lease until child teardown and durable IO have actually ended.
use crate::{model_download_recipe::Recipe, model_inspection::Inspector, Storage};
use serde::Serialize;
use tauri::{Manager, WebviewWindow};

pub(crate) fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn id(value: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err("モデル操作IDが不正です".into())
    }
}
fn binding(operation: &str, recipe_hash: &str) -> Result<(), String> {
    id(operation)?;
    if hash(recipe_hash) {
        Ok(())
    } else {
        Err("確認した定義のSHAが不正です".into())
    }
}
fn main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("モデル取得はメイン画面だけで利用できます".into())
    }
}
fn json(value: impl Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

async fn prepare<F>(
    window: WebviewWindow,
    token: String,
    work: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(&tauri::AppHandle, &dyn Fn() -> bool) -> Result<serde_json::Value, String>
        + Send
        + 'static,
{
    main(&window)?;
    let guard = window.state::<Inspector>().begin(&token)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::AnalysisController::with_model_preparation(
            &app,
            Some(&token),
            &|| guard.cancelled(),
            |lease| work(&app, &|| guard.cancelled() || lease.cancelled()),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn start_model_download(
    window: WebviewWindow,
    token: String,
    definition: String,
    expected_recipe_hash: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    binding(&token, &expected_recipe_hash)?;

    {
        // No filesystem or network before the exact reviewed definition and
        // future placement-record budget pass. begin also rejects late starts.
        let recipe = Recipe::parse(definition.as_bytes())?;
        if recipe.review()?.recipe_hash != expected_recipe_hash {
            return Err("確認した取得定義と一致しません".into());
        }
        let operation = token.clone();
        prepare(window, token, move |app, cancelled| {
            use tauri::Emitter;
            let root = app.path().app_data_dir().map_err(|e| e.to_string())?.join("model-downloads");
            let mut last = None::<std::time::Instant>;
            let bundled = app.path().resource_dir().map_err(|e| e.to_string())?.join(if cfg!(windows) { "windows-runtime" } else { "asr-runtime" });
            let reuse_root = root.parent().ok_or("モデル管理領域がありません")?.join("models");
            let reusable = app.state::<crate::storage::Storage>().list_model_placements()?;
            let receipt = crate::model_download::download_platform(&root, &bundled, Some((&reuse_root, &reusable)), &operation, &recipe, &expected_recipe_hash, &|| cancelled(), &mut |received, total| {
                if received == 0 || received == total || last.is_none_or(|time| time.elapsed().as_millis() >= 200) {
                    last = Some(std::time::Instant::now());
                    // Progress is informational, never a success receipt. Lost
                    // listeners do not trigger retries or weaken IO checks.
                    let _ = app.emit_to("main", "model-download-progress", serde_json::json!({"operationId": operation, "recipeHash": expected_recipe_hash, "receivedBytes": received, "totalBytes": total}));
                }
            })?;
            json(receipt)
        }).await
    }
}

#[tauri::command]
pub(crate) fn get_model_download_lifetime(
    window: WebviewWindow,
    token: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    let (active, cancelled) = window.state::<Inspector>().status(&token)?;
    // Inactive alone does not mean a delayed start IPC cannot arrive. Recovery
    // first cancels this exact token and checks its retained tombstone.
    Ok(serde_json::json!({"active": active, "cancelled": cancelled}))
}

#[tauri::command]
pub(crate) async fn get_model_download_catalog(
    window: WebviewWindow,
    token: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    id(&token)?;

    {
        prepare(window, token, move |app, cancelled| {
            let root = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("model-downloads");
            json(crate::model_download::catalog::catalog(&root, &|| {
                cancelled()
            })?)
        })
        .await
    }
}

#[tauri::command]
pub(crate) async fn read_model_download(
    window: WebviewWindow,
    token: String,
    download_id: String,
    expected_recipe_hash: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    binding(&download_id, &expected_recipe_hash)?;
    if window.state::<Inspector>().status(&download_id)?.0 {
        return Err("元の取得処理の停止完了を待ってください".into());
    }

    {
        prepare(window, token, move |app, cancelled| {
            let root = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("model-downloads");
            json(crate::model_download::readback(
                &root,
                &download_id,
                &expected_recipe_hash,
                &|| cancelled(),
            )?)
        })
        .await
    }
}

#[tauri::command]
pub(crate) async fn place_downloaded_model(
    window: WebviewWindow,
    token: String,
    download_id: String,
    expected_recipe_hash: String,
    expected_review_hash: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    binding(&download_id, &expected_recipe_hash)?;
    if !hash(&expected_review_hash) {
        return Err("素材の確認SHAが不正です".into());
    }

    {
        let package = token.clone();
        prepare(window, token, move |app, cancelled| {
            let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
            json(crate::model_download::place(
                &app.state::<Storage>(),
                &root.join("models"),
                &root.join("model-downloads"),
                &download_id,
                &expected_recipe_hash,
                &package,
                &expected_review_hash,
                &|| cancelled(),
            )?)
        })
        .await
    }
}

#[tauri::command]
pub(crate) fn get_downloaded_model_placement_result(
    window: WebviewWindow,
    package_id: String,
    download_id: String,
    expected_recipe_hash: String,
    expected_review_hash: String,
) -> Result<serde_json::Value, String> {
    main(&window)?;
    id(&package_id)?;
    binding(&download_id, &expected_recipe_hash)?;
    if !hash(&expected_review_hash) {
        return Err("素材の確認SHAが不正です".into());
    }
    // Observe lifetime BEFORE the DB read: seeing active=false cannot race a
    // pre-existing worker's later commit. Cancellation tombstone fences late IPC.
    let (active, cancelled) = window.state::<Inspector>().status(&package_id)?;
    let receipt = window.state::<Storage>().downloaded_placement_result(
        &package_id,
        &download_id,
        &expected_recipe_hash,
        &expected_review_hash,
    )?;
    Ok(serde_json::json!({"active": active, "cancelled": cancelled, "receipt": receipt}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_rejects_noncanonical_operation_and_hash_before_any_io() {
        let operation = uuid::Uuid::new_v4().to_string();
        assert!(binding(&operation, &"a".repeat(64)).is_ok());
        for value in ["../models", "", &operation.to_uppercase()] {
            assert!(binding(value, &"a".repeat(64)).is_err());
        }
        for value in ["", &"a".repeat(63), &"A".repeat(64), &"g".repeat(64)] {
            assert!(binding(&operation, value).is_err());
        }
    }
}

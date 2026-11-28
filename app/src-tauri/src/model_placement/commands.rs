use crate::model_placement;
use crate::storage::Storage;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_model_placements(
    window: WebviewWindow,
) -> Result<Vec<model_placement::CatalogEntry>, mon_contracts::AppError> {
    let result: Result<Vec<model_placement::CatalogEntry>, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read model placements".into());
        }
        window.state::<Storage>().list_model_placements()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_model_placements", message))
}

use crate::model_placement;
use crate::storage::Storage;
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_model_placements(
    window: WebviewWindow,
) -> Result<Vec<model_placement::CatalogEntry>, mon_contracts::AppError> {
    let result: Result<Vec<model_placement::CatalogEntry>, String> = (|| {
        require_main_window(&window, "Only the main window can read model placements")?;
        window.state::<Storage>().list_model_placements()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_model_placements", message))
}

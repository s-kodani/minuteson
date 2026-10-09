import Foundation

/// Presentation only: keep status codes and evidence unchanged. Do not infer
/// recovered audio, acquisition closure, a loss bound or permission to delete.
enum CaptureFailureGuide {
  static func message(_ code: String) -> String {
    let reason: String
    switch code {
    case "raw_segment_storage_full":
      reason = "録音の保存先の空き容量またはクォータが不足しています。空き容量を確保してください。"
    case "raw_segment_storage_io_failed":
      reason = "録音の保存先への書込み・同期に失敗しています。保存先の接続・アクセス状態を確認してください。"
    case "capture_clock_failed":
      reason = "録音の取得時刻と保存音声の対応を検証できず、保存を継続できません。"
    case "capture_audio_write_failed", "raw_segment_write_or_sync_failed",
      "raw_segment_final_sync_failed":
      reason = "録音の書込み・確定を確認できず、保存を継続できません。保存先と録音状態を確認してください。"
    default: return code
    }
    return reason
      + "録音の停止を確認してから「中断録音の部分救出」で確定範囲を確認してください。音声は自動削除せず、正常終了・未確定末尾の保存は保証しません。（\(code)）"
  }
}

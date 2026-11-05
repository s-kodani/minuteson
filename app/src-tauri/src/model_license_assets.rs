//! Verbatim notice bytes compiled into the app; renderer paths never select files.
pub(super) fn embedded(sha256: &str) -> Option<&'static [u8]> {
    match sha256 {
        "0e07b95f3a8d6230037707c5c4a2b554d12c4cb67369669ac255635528ffcee2" => Some(include_bytes!("../../../third-party/model-licenses/0e07b95f3a8d6230037707c5c4a2b554d12c4cb67369669ac255635528ffcee2.txt")),
        "14d7016ad68e7394d6e6b78d96cc2ae431c905287b89674cfdf021e79e62b8ba" => Some(include_bytes!("../../../third-party/model-licenses/14d7016ad68e7394d6e6b78d96cc2ae431c905287b89674cfdf021e79e62b8ba.txt")),
        "21fd967098804f33fc84e803fb0e5ab7666d71801f4027cf28a65e7af09c1758" => Some(include_bytes!("../../../third-party/model-licenses/21fd967098804f33fc84e803fb0e5ab7666d71801f4027cf28a65e7af09c1758.txt")),
        "2e63e9a38b6e8fc0c7bc37ce174caca1862870856c6daf5697cfb785e925520b" => Some(include_bytes!("../../../third-party/model-licenses/2e63e9a38b6e8fc0c7bc37ce174caca1862870856c6daf5697cfb785e925520b.txt")),
        "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c" => Some(include_bytes!("../../../third-party/model-licenses/2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c.txt")),
        "5de36594c10839788a8c589443a8ef9d8b8d17c65a1b5807206ae037fc36c6bd" => Some(include_bytes!("../../../third-party/model-licenses/5de36594c10839788a8c589443a8ef9d8b8d17c65a1b5807206ae037fc36c6bd.txt")),
        "6c1077880c654e84e103c34d2f7560e058ee62fecd2bc868db0249641c5d247e" => Some(include_bytes!("../../../third-party/model-licenses/6c1077880c654e84e103c34d2f7560e058ee62fecd2bc868db0249641c5d247e.txt")),
        "7233e8200b31f6e3d28f5c2df0e00b3734ccef458ac167ca85140db26b352be8" => Some(include_bytes!("../../../third-party/model-licenses/7233e8200b31f6e3d28f5c2df0e00b3734ccef458ac167ca85140db26b352be8.txt")),
        "7debad4c9430f3310ad6d119fce385787c1c19f3ebc0cabe685a48dbe72a4de0" => Some(include_bytes!("../../../third-party/model-licenses/7debad4c9430f3310ad6d119fce385787c1c19f3ebc0cabe685a48dbe72a4de0.txt")),
        "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d" => Some(include_bytes!("../../../third-party/model-licenses/94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d.txt")),
        "b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d" => Some(include_bytes!("../../../third-party/model-licenses/b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d.txt")),
        "bbedc3fda3305820b977265f01b8619d87570a6739de3a5582c3464840f1e57a" => Some(include_bytes!("../../../third-party/model-licenses/bbedc3fda3305820b977265f01b8619d87570a6739de3a5582c3464840f1e57a.txt")),
        "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4" => Some(include_bytes!("../../../third-party/model-licenses/c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4.txt")),
        "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30" => Some(include_bytes!("../../../third-party/model-licenses/cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30.txt")),
        _ => None,
    }
}

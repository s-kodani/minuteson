use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    for path in [
        "src",
        "cbindgen.toml",
        "include/mon_capture.h",
        "../mon-core/src/capture",
        "../mon-core/src/media",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=MON_CAPTURE_UPDATE_HEADER");
    let bindings = cbindgen::Builder::new()
        .with_crate(&root)
        .with_config(
            cbindgen::Config::from_file(root.join("cbindgen.toml"))
                .expect("capture ABI configuration"),
        )
        .generate()
        .expect("generate capture ABI header");
    let mut bytes = Vec::new();
    bindings.write(&mut bytes);
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("mon_capture.h"),
        &bytes,
    )
    .expect("write generated capture header");
    let checked = root.join("include/mon_capture.h");
    if env::var_os("MON_CAPTURE_UPDATE_HEADER").as_deref() == Some(std::ffi::OsStr::new("1")) {
        fs::write(&checked, &bytes).expect("update capture ABI header");
    } else {
        if fs::read(&checked).expect("checked capture ABI header") != bytes {
            panic!("capture ABI header is stale; regenerate explicitly with MON_CAPTURE_UPDATE_HEADER=1 cargo build -p mon-capture-ffi");
        }
    }
}

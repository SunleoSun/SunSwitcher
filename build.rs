use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const RUNTIME_ASSETS: [&str; 2] = ["assets/icon.png", "assets/clipboard.ico"];

fn main() {
    for asset in RUNTIME_ASSETS {
        println!("cargo:rerun-if-changed={asset}");
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let profile_dir = profile_dir_from_out_dir(&out_dir)
        .expect("Cargo OUT_DIR must be inside <target>/<profile>/build/<package>/out");
    let destination_dir = profile_dir.join("assets");
    fs::create_dir_all(&destination_dir).expect("could not create runtime assets directory");

    for asset in RUNTIME_ASSETS {
        let source = Path::new(asset);
        let file_name = source
            .file_name()
            .expect("runtime asset path must end in a file name");
        fs::copy(source, destination_dir.join(file_name))
            .unwrap_or_else(|error| panic!("could not package runtime asset {asset}: {error}"));
    }
}

fn profile_dir_from_out_dir(out_dir: &Path) -> Option<PathBuf> {
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}

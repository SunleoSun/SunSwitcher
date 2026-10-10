const EMBEDDED_ASSETS: [&str; 2] = ["assets/icon.png", "assets/clipboard.ico"];

fn main() {
    for asset in EMBEDDED_ASSETS {
        println!("cargo:rerun-if-changed={asset}");
    }
}

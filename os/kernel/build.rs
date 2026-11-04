// Links the kernel with its own layout (kernel.ld); for a test build, also its script and certificate authority.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-arg-bins=-T{dir}/kernel.ld");
    println!("cargo:rerun-if-changed=kernel.ld");

    // A test build that answers from a script in place of a model (CHATCOMPUTER_TEST=chat, by check.py) has the
    // script named by CHATCOMPUTER_TEST_SCRIPT (JSON: see assistant.rs) built in. Any other build gets nothing here.
    println!("cargo:rerun-if-env-changed=CHATCOMPUTER_TEST");
    println!("cargo:rerun-if-env-changed=CHATCOMPUTER_TEST_SCRIPT");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("test_script.json");
    let script = match (std::env::var("CHATCOMPUTER_TEST").as_deref(), std::env::var("CHATCOMPUTER_TEST_SCRIPT")) {
        (Ok("chat"), Ok(path)) => {
            println!("cargo:rerun-if-changed={path}");
            std::fs::read(&path).unwrap_or_else(|e| panic!("CHATCOMPUTER_TEST_SCRIPT {path}: {e}"))
        }
        _ => Vec::new(),
    };
    std::fs::write(out, script).expect("writing the test script");

    // A test build that reads the web from a stand-in on the computer running the virtual PC (check.py --web), or
    // reaches Meta's Model API through a relay there (check.py --meta), trusts that test's own certificate authority,
    // named by CHATCOMPUTER_TEST_CA (its certificate, DER), instead of Mozilla's (web.rs). Any other build gets
    // nothing here.
    println!("cargo:rerun-if-env-changed=CHATCOMPUTER_TEST_CA");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("test_ca.der");
    let ca = match (std::env::var("CHATCOMPUTER_TEST").as_deref(), std::env::var("CHATCOMPUTER_TEST_CA")) {
        (Ok("chat" | "meta" | "listening"), Ok(path)) => {
            println!("cargo:rerun-if-changed={path}");
            std::fs::read(&path).unwrap_or_else(|e| panic!("CHATCOMPUTER_TEST_CA {path}: {e}"))
        }
        _ => Vec::new(),
    };
    std::fs::write(out, ca).expect("writing the test authority");
}

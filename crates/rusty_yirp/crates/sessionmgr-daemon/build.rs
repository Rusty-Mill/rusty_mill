//! Embeds `sessionmgr.exe.manifest` into the built `sessionmgr` binary.
//!
//! Gated on `CARGO_CFG_WINDOWS` rather than `cfg!(windows)`: a build
//! script runs on the *host*, and this project cross-compiles the daemon
//! for a `--target x86_64-pc-windows-msvc` check from Linux CI (see
//! `docs/decisions/0001-async-runtime-rusty-tokio.md`). `CARGO_CFG_*`
//! reflects the *target* cfg, which is the question that actually
//! matters here -- a manifest is meaningless for any other target, and
//! `embed_manifest_file` itself only knows how to talk to a Windows
//! linker.
//!
//! Uses `embed-manifest` rather than `winres`/`embed_resource` because it
//! does not shell out to `rc.exe`: on MSVC it passes `/MANIFEST` options
//! straight to `LINK.EXE`, so the build never depends on the Windows SDK
//! being discoverable on `PATH`.

fn main() {
    println!("cargo:rerun-if-changed=sessionmgr.exe.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    embed_manifest::embed_manifest_file("sessionmgr.exe.manifest")
        .expect("unable to embed sessionmgr.exe.manifest");
}

#!/usr/bin/env python3
"""Check the public API from isolated Cargo consumers, including TLS feature unification.

Run from any directory: python3 crates/foundation/rusty_aead/scripts/check_features.py
Uses the repository's selected Rust toolchain; no RUSTFLAGS/cfg provider selection is needed.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
AEAD = ROOT / "crates/foundation/rusty_aead"
TLS = ROOT / "crates/libs/net/rusty_tls"
AES_API = ("Aes128Gcm", "Aes256Gcm", "GCM_NONCE_LEN", "GCM_TAG_LEN")
COMMON = """use rusty_aead::{ChaCha20Poly1305, Poly1305, Error, KEY_LEN, NONCE_LEN, TAG_LEN};
fn main() {
    let _ = ChaCha20Poly1305::new(&[0; KEY_LEN]);
    let _ = core::mem::size_of::<Poly1305>();
    let _ = core::mem::size_of::<Error>();
    let _ = (NONCE_LEN, TAG_LEN);
}
"""
AES = "use rusty_aead::{" + ", ".join(AES_API) + "};\n" + COMMON


def check(
    directory: Path,
    dependency: str,
    source: str,
    features: tuple[str, ...] = (),
    missing: tuple[str, ...] = (),
) -> None:
    """Require a clean compile or the specific missing public API diagnostics."""
    (directory / "Cargo.toml").write_text(f'''[package]
name = "aead-feature-consumer"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
rusty_aead = {{ path = "{AEAD}", {dependency} }}
[features]
native = ["dep:rusty_tls", "rusty_tls/native-crypto"]
[dependencies.rusty_tls]
path = "{TLS}"
optional = true
''')
    (directory / "src/main.rs").write_text(source)
    command = ["cargo", "check", "--manifest-path", str(directory / "Cargo.toml")]
    if features:
        command += ["--features", ",".join(features)]
    done = subprocess.run(command, cwd=directory, text=True, capture_output=True)
    if missing:
        assert done.returncode != 0, "AES API unexpectedly available without opt-in"
        for name in missing:
            assert f"no `{name}` in the root" in done.stderr, done.stderr
    else:
        assert done.returncode == 0, done.stderr


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="aead-features-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        # Pin to the repo toolchain even though the temporary consumer lives outside it.
        (directory / "rust-toolchain.toml").write_text((ROOT / "rust-toolchain.toml").read_text())
        for dependency in ("default-features = true", "default-features = false"):
            check(directory, dependency, COMMON)
            check(directory, dependency, AES, missing=AES_API)
            print(f"PASS: {dependency}: common API present, all four AES exports absent", flush=True)
        check(directory, 'features = ["aes-gcm"]', AES)
        print("PASS: explicit aes-gcm exposes all four AES exports", flush=True)
        check(directory, "default-features = false", AES, features=("native",))
        print("PASS: TLS native-crypto unifies aes-gcm into the direct AEAD consumer", flush=True)

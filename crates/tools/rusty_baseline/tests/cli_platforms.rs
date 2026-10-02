use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rusty-baseline-platforms-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn products(&self, contents: &str) -> PathBuf {
        let path = self.path("products.txt");
        fs::write(&path, contents).unwrap();
        path
    }

    fn workspace(&self, bins: &[&str]) {
        let mut manifest = String::from(
            "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        );
        for bin in bins {
            manifest.push_str(&format!(
                "\n[[bin]]\nname = \"{bin}\"\npath = \"src/main.rs\"\n"
            ));
        }
        fs::create_dir_all(self.path("src")).unwrap();
        fs::write(self.path("Cargo.toml"), manifest).unwrap();
        fs::write(self.path("src/main.rs"), "fn main() {}\n").unwrap();
    }

    fn compile_helper(&self, name: &str, body: &str, directory: &Path) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        let source = self.path(&format!("{name}.rs"));
        fs::write(&source, body).unwrap();
        let output = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let status = Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&output)
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        output
    }

    fn run(&self, products: &Path, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rusty_baseline"))
            .current_dir(&self.root)
            .args(["--products", products.to_str().unwrap(), "--work"])
            .arg(self.path("work"))
            .args(extra)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn other_os() -> &'static str {
    if std::env::consts::OS == "linux" {
        "windows"
    } else {
        "linux"
    }
}

fn marker_helper(marker: &Path, exit: i32) -> String {
    format!(
        "fn main() {{ std::fs::write({marker:?}, b\"ran\").unwrap(); std::process::exit({exit}); }}"
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn all_unsupported_normal_and_prebuilt_never_invoke_cargo_or_a_binary() {
    for prebuilt in [false, true] {
        let fixture = Fixture::new();
        let cargo_marker = fixture.path("cargo-invoked");
        let cargo_dir = fixture.path("trap-cargo");
        let trap_cargo =
            fixture.compile_helper("cargo-trap", &marker_helper(&cargo_marker, 91), &cargo_dir);
        let products = fixture.products(&format!(
            "absent trap exit @platform={} --help\n",
            other_os()
        ));
        let prebuilt_dir = fixture.path("prebuilt");
        let binary_marker = fixture.path("binary-ran");
        fixture.compile_helper(
            "trap",
            &marker_helper(&binary_marker, 92),
            &prebuilt_dir.join("release"),
        );

        let mut command = Command::new(env!("CARGO_BIN_EXE_rusty_baseline"));
        command
            .current_dir(&fixture.root)
            .env("CARGO", trap_cargo)
            .args(["--products", products.to_str().unwrap(), "--work"])
            .arg(fixture.path("work"));
        if prebuilt {
            command.arg("--prebuilt").arg(&prebuilt_dir);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let table = stdout(&output);
        assert!(table.contains(&format!(
            "unsupported on {} (allowed: {}); skipped",
            std::env::consts::OS,
            other_os()
        )));
        assert!(
            !cargo_marker.exists(),
            "unsupported selection invoked Cargo"
        );
        assert!(!binary_marker.exists(), "unsupported prebuilt binary ran");
    }
}

#[test]
fn mixed_prebuilt_selection_preserves_rows_and_runs_only_the_eligible_product() {
    let fixture = Fixture::new();
    fixture.workspace(&["good"]);
    let marker = fixture.path("good-ran");
    let trap_marker = fixture.path("trap-ran");
    let target = fixture.path("prebuilt");
    fixture.compile_helper("good", &marker_helper(&marker, 0), &target.join("release"));
    fixture.compile_helper(
        "unsupported-after",
        &marker_helper(&trap_marker, 0),
        &target.join("release"),
    );
    let products = fixture.products(&format!(
        "absent-before unsupported-before exit @platform={}\nfixture good exit\nabsent-after unsupported-after exit @platform={}\n",
        other_os(),
        other_os()
    ));
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table = stdout(&output);
    let before = table.find("`unsupported-before`").unwrap();
    let good = table.find("`good`").unwrap();
    let after = table.find("`unsupported-after`").unwrap();
    assert!(before < good && good < after);
    assert!(marker.exists(), "eligible prebuilt fixture did not run");
    assert!(!trap_marker.exists(), "unsupported fixture ran");
}

#[test]
fn an_eligible_missing_binary_is_an_error_and_later_measurement_completes() {
    let fixture = Fixture::new();
    fixture.workspace(&["missing", "good", "nonzero"]);
    let good_marker = fixture.path("good-ran");
    let target = fixture.path("prebuilt");
    fixture.compile_helper(
        "good",
        &marker_helper(&good_marker, 0),
        &target.join("release"),
    );
    fixture.compile_helper(
        "nonzero",
        "fn main() { std::process::exit(7); }",
        &target.join("release"),
    );
    let products =
        fixture.products("fixture missing exit\nfixture good exit\nfixture nonzero exit\n");
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table = stdout(&output);
    let missing = table
        .lines()
        .find(|line| line.starts_with("| `missing`"))
        .unwrap();
    assert!(missing.contains("run: no binary at"));
    assert!(!missing.contains("unsupported"));
    assert!(good_marker.exists(), "later eligible fixture did not run");
    let nonzero = table
        .lines()
        .find(|line| line.starts_with("| `nonzero`"))
        .unwrap();
    assert!(nonzero.contains("ended Exited(7)"));
}

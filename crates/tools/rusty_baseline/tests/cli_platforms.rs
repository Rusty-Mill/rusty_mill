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
            "[workspace]\n\n[package]\nname = \"fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        );
        for bin in bins {
            manifest.push_str(&format!(
                "\n[[bin]]\nname = \"{bin}\"\npath = \"src/main.rs\"\n"
            ));
        }
        fs::create_dir_all(self.path("src")).unwrap();
        fs::write(self.path("Cargo.toml"), manifest).unwrap();
        fs::write(
            self.path("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"fixture\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
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
fn explicitly_excluded_host_never_invokes_cargo_or_an_existing_stub() {
    let fixture = Fixture::new();
    let cargo_marker = fixture.path("cargo-invoked");
    let cargo_dir = fixture.path("trap-cargo");
    let trap_cargo =
        fixture.compile_helper("cargo-trap", &marker_helper(&cargo_marker, 91), &cargo_dir);
    let products = fixture.products(&format!(
        "absent ts-daemon exit @unsupported={} --help\n",
        std::env::consts::OS
    ));
    let prebuilt_dir = fixture.path("prebuilt");
    let stub_marker = fixture.path("stub-ran");
    fixture.compile_helper(
        "ts-daemon",
        &marker_helper(&stub_marker, 0),
        &prebuilt_dir.join("release"),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_rusty_baseline"))
        .current_dir(&fixture.root)
        .env("CARGO", trap_cargo)
        .args(["--products", products.to_str().unwrap(), "--work"])
        .arg(fixture.path("work"))
        .arg("--prebuilt")
        .arg(&prebuilt_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout(&output).contains(&format!(
        "unsupported on {} (excluded: {}); skipped",
        std::env::consts::OS,
        std::env::consts::OS
    )));
    assert!(!cargo_marker.exists(), "excluded selection invoked Cargo");
    assert!(!stub_marker.exists(), "excluded stub binary ran");
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
    assert_eq!(output.status.code(), Some(1), "{table}");
}

#[test]
fn every_exit_sample_is_validated_and_later_products_still_run() {
    let fixture = Fixture::new();
    fixture.workspace(&["warmup", "early", "last", "good"]);
    let target = fixture.path("prebuilt");
    for (name, failed_run) in [("warmup", 0), ("early", 1), ("last", 3)] {
        let counter = fixture.path(&format!("{name}-count"));
        fixture.compile_helper(
            name,
            &format!(
                r#"fn main() {{
                    let path = {counter:?};
                    let n: usize = std::fs::read_to_string(path)
                        .unwrap_or_else(|_| "0".into()).parse().unwrap();
                    std::fs::write(path, (n + 1).to_string()).unwrap();
                    if n == {failed_run} {{ std::process::exit(7); }}
                }}"#
            ),
            &target.join("release"),
        );
    }
    let marker = fixture.path("good-ran");
    fixture.compile_helper("good", &marker_helper(&marker, 0), &target.join("release"));
    let products = fixture.products(
        "fixture warmup exit\nfixture early exit\nfixture last exit\nfixture good exit\n",
    );
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "3"],
    );
    let table = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{table}");
    for (name, phase) in [
        ("warmup", "warm-up"),
        ("early", "timed run 1"),
        ("last", "timed run 3"),
    ] {
        let row = table
            .lines()
            .find(|line| line.starts_with(&format!("| `{name}`")))
            .unwrap();
        assert!(row.contains(phase), "{row}");
        assert!(row.contains("ended Exited(7); expected exit 0"), "{row}");
        assert!(row.contains("failed"), "{row}");
    }
    assert!(marker.exists());
    assert!(table.find("`last`").unwrap() < table.find("`good`").unwrap());
}

#[test]
fn declared_nonzero_exit_is_success_and_not_passed_to_the_product() {
    let fixture = Fixture::new();
    fixture.workspace(&["expected"]);
    let target = fixture.path("prebuilt");
    fixture.compile_helper(
        "expected",
        r#"fn main() {
            assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), ["--help"]);
            std::process::exit(2);
        }"#,
        &target.join("release"),
    );
    for declaration in [
        "@expect-exit=2".to_owned(),
        format!("@expect-exit={}:2", std::env::consts::OS),
    ] {
        let products = fixture.products(&format!("fixture expected exit {declaration} --help\n"));
        let output = fixture.run(
            &products,
            &["--prebuilt", target.to_str().unwrap(), "--runs", "2"],
        );
        let table = stdout(&output);
        assert!(output.status.success(), "{table}");
        assert!(table.contains("ended Exited(2)"), "{table}");
        assert!(!table.contains("failed"), "{table}");
    }
    let products = fixture.products("fixture expected exit @expect-exit=7 --help\n");
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "1"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("ended Exited(2); expected exit 7"));
}

#[test]
fn dependency_failure_keeps_successful_runtime_measurement() {
    let fixture = Fixture::new();
    fixture.workspace(&["good"]);
    // metadata --no-deps succeeds, but cargo tree --locked must fail.
    fs::remove_file(fixture.path("Cargo.lock")).unwrap();
    let target = fixture.path("prebuilt");
    let marker = fixture.path("good-ran");
    fixture.compile_helper("good", &marker_helper(&marker, 0), &target.join("release"));
    let products = fixture.products("fixture good exit\n");
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "1"],
    );
    let table = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{table}");
    assert!(table.contains("deps:"), "{table}");
    assert!(table.contains(" ms |"), "{table}");
    assert!(marker.exists());
}

#[test]
fn failed_build_keeps_dependency_measurement_and_returns_failure() {
    let fixture = Fixture::new();
    fixture.workspace(&["broken"]);
    fs::write(
        fixture.path("src/main.rs"),
        "compile_error!(\"owned fixture build failure\");\nfn main() {}\n",
    )
    .unwrap();
    let products = fixture.products("fixture broken exit\n");
    let output = fixture.run(&products, &["--runs", "1"]);
    let table = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{table}");
    assert!(table.contains("| 0 + 0 | failed |"), "{table}");
    assert!(table.contains("build:"), "{table}");
    assert!(!fixture.path("work/target/broken").exists());
}

#[test]
fn spawn_failure_keeps_later_rows_and_is_not_unsupported() {
    let fixture = Fixture::new();
    fixture.workspace(&["invalid", "good"]);
    let target = fixture.path("prebuilt");
    let marker = fixture.path("good-ran");
    fixture.compile_helper("good", &marker_helper(&marker, 0), &target.join("release"));
    fs::create_dir(
        target
            .join("release")
            .join(format!("invalid{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
    let products = fixture.products("fixture invalid exit\nfixture good exit\n");
    let output = fixture.run(
        &products,
        &["--prebuilt", target.to_str().unwrap(), "--runs", "1"],
    );
    let table = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{table}");
    assert!(table.contains("run: warm-up starting"), "{table}");
    assert!(!table.contains("unsupported"), "{table}");
    assert!(marker.exists());
}

#[test]
fn idle_early_exit_fails_but_intentional_teardown_succeeds() {
    let fixture = Fixture::new();
    fixture.workspace(&["early", "idle", "good"]);
    let target = fixture.path("prebuilt");
    let marker = fixture.path("good-ran");
    fixture.compile_helper("good", &marker_helper(&marker, 0), &target.join("release"));
    fixture.compile_helper("early", "fn main() {}", &target.join("release"));
    fixture.compile_helper(
        "idle",
        "fn main() { loop { std::thread::park(); } }",
        &target.join("release"),
    );
    for (bin, success) in [("early", false), ("idle", true)] {
        let products = fixture.products(&format!("fixture {bin} idle\nfixture good exit\n"));
        let output = fixture.run(
            &products,
            &[
                "--prebuilt",
                target.to_str().unwrap(),
                "--settle",
                "1",
                "--runs",
                "1",
            ],
        );
        let table = stdout(&output);
        assert_eq!(output.status.success(), success, "{table}");
        assert_eq!(table.contains("before settling"), !success, "{table}");
        assert!(table.find(&format!("`{bin}`")).unwrap() < table.find("`good`").unwrap());
        assert!(marker.exists(), "later product did not run");
        fs::remove_file(&marker).unwrap();
    }
}

//! The product list: which binaries the baseline measures, and how to run
//! each one.

/// How a product is run to measure startup and memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Runs to completion (e.g. `--help`): startup is its wall time.
    Exit,
    /// Long-running (a server or daemon): sampled once settled, then killed.
    Idle,
}

/// An operating system on which a product can be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Macos,
}

impl Platform {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "linux" => Ok(Self::Linux),
            "windows" => Ok(Self::Windows),
            "macos" => Ok(Self::Macos),
            _ => Err(format!(
                "unknown platform `{value}` (expected `linux`, `windows`, or `macos`)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
        }
    }
}

/// One product: a binary target of a workspace package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub package: String,
    /// Cargo features to enable, comma-separated as `--features` takes them.
    pub features: Option<String>,
    pub bin: String,
    pub mode: Mode,
    /// The only supported OS, or `None` when the product is unrestricted.
    pub platform: Option<Platform>,
    /// The explicitly unsupported OS, or `None` when no OS is excluded.
    pub unsupported: Option<Platform>,
    /// Environment variables set for the run, from leading `KEY=value`s.
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
}

/// Parses the product list: one product per line,
/// `<package>[:<features>] <bin> <exit|idle> [@platform=<os>|@unsupported=<os>] [KEY=value...] [args...]`.
/// The optional policy declaration accepts `linux`, `windows`, or `macos`
/// and must immediately follow the mode. `@platform` is an allowlist while
/// `@unsupported` excludes only the named OS. An omitted declaration is unrestricted.
/// Leading `KEY=value`s (an uppercase name) set the run's environment, as
/// in a shell. Blank lines and `#` comments are skipped.
pub fn parse(text: &str) -> Result<Vec<Product>, String> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.split('#').next().unwrap_or("").trim()))
        .filter(|(_, line)| !line.is_empty())
        .map(|(number, line)| parse_line(line).map_err(|error| format!("line {number}: {error}")))
        .collect()
}

fn parse_line(line: &str) -> Result<Product, String> {
    let mut fields = line.split_whitespace();
    let (Some(package), Some(bin), Some(mode)) = (fields.next(), fields.next(), fields.next())
    else {
        return Err(format!(
            "expected `<package> <bin> <exit|idle> [args...]`, got `{line}`"
        ));
    };
    let mode = match mode {
        "exit" => Mode::Exit,
        "idle" => Mode::Idle,
        other => {
            return Err(format!(
                "unknown mode `{other}` (expected `exit` or `idle`)"
            ));
        }
    };
    let (package, features) = match package.split_once(':') {
        Some((package, features)) => (package, Some(features.to_owned())),
        None => (package, None),
    };
    let mut fields = fields.peekable();
    let (platform, unsupported) = match fields.peek().copied() {
        Some(field) if field.starts_with("@platform") || field.starts_with("@unsupported") => {
            let (prefix, kind) = if field.starts_with("@platform") {
                ("@platform=", "platform")
            } else {
                ("@unsupported=", "unsupported")
            };
            let value = field.strip_prefix(prefix).ok_or_else(|| {
                format!("malformed {kind} declaration `{field}` (expected `{prefix}<os>`)")
            })?;
            if value.is_empty() {
                return Err(format!(
                    "malformed {kind} declaration `{prefix}` (missing OS)"
                ));
            }
            fields.next();
            let value = Platform::parse(value)?;
            if kind == "platform" {
                (Some(value), None)
            } else {
                (None, Some(value))
            }
        }
        _ => (None, None),
    };
    if let Some(field) = fields
        .clone()
        .find(|field| field.starts_with("@platform") || field.starts_with("@unsupported"))
    {
        return Err(format!(
            "platform policy declaration `{field}` must immediately follow the mode"
        ));
    }
    let mut env = Vec::new();
    while let Some((key, value)) = fields.peek().and_then(|field| env_assignment(field)) {
        env.push((key.to_owned(), value.to_owned()));
        fields.next();
    }
    Ok(Product {
        package: package.to_owned(),
        features,
        bin: bin.to_owned(),
        mode,
        platform,
        unsupported,
        env,
        args: fields.map(str::to_owned).collect(),
    })
}

fn env_assignment(field: &str) -> Option<(&str, &str)> {
    let (key, value) = field.split_once('=')?;
    let is_name = !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    is_name.then_some((key, value))
}

impl Product {
    /// Whether this product is eligible on `os` (`std::env::consts::OS` form).
    pub fn supports_os(&self, os: &str) -> bool {
        self.platform.is_none_or(|platform| platform.name() == os)
            && self
                .unsupported
                .is_none_or(|platform| platform.name() != os)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes_features_args_and_comments() {
        let text =
            "# header\n\nrush rush exit -c true\ndb:server,client memory_server idle # note\n";
        let products = parse(text).unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].mode, Mode::Exit);
        assert_eq!(products[0].args, ["-c", "true"]);
        assert_eq!(products[0].features, None);
        assert_eq!(products[1].package, "db");
        assert_eq!(products[1].features.as_deref(), Some("server,client"));
        assert_eq!(products[1].mode, Mode::Idle);
        assert!(products[1].args.is_empty());
        assert_eq!(products[0].platform, None);
        assert_eq!(products[0].unsupported, None);
    }

    #[test]
    fn exclusion_policy_excludes_only_the_named_platform() {
        let product = parse("daemon daemon exit @unsupported=windows --help\n")
            .unwrap()
            .remove(0);
        assert_eq!(product.platform, None);
        assert_eq!(product.unsupported, Some(Platform::Windows));
        assert_eq!(product.args, ["--help"]);
        assert!(product.supports_os("linux"));
        assert!(product.supports_os("macos"));
        assert!(!product.supports_os("windows"));
    }

    #[test]
    fn parses_platform_and_checks_it_deterministically() {
        let product = parse("agent:api agent exit @platform=linux --help\n")
            .unwrap()
            .remove(0);
        assert_eq!(product.features.as_deref(), Some("api"));
        assert_eq!(product.platform, Some(Platform::Linux));
        assert_eq!(product.args, ["--help"]);
        assert!(product.supports_os("linux"));
        assert!(!product.supports_os("windows"));
    }

    #[test]
    fn malformed_platform_policy_declarations_have_line_context() {
        for text in [
            "agent agent exit @platform=\n",
            "agent agent exit @platform-linux\n",
            "agent agent exit @platform=plan9\n",
            "agent agent exit --help @platform=linux\n",
            "agent agent exit @unsupported=\n",
            "agent agent exit @unsupported-windows\n",
            "agent agent exit @unsupported=plan9\n",
            "agent agent exit @platform=linux @unsupported=windows\n",
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.starts_with("line 1:"), "{error}");
            assert!(
                error.contains("platform") || error.contains("unsupported"),
                "{error}"
            );
        }
    }

    #[test]
    fn leading_assignments_are_env_and_later_ones_are_args() {
        let product = parse("tick rusty_tick idle TOKEN=a=b DIR=x --flag KEY=arg\n")
            .unwrap()
            .remove(0);
        let env = [
            ("TOKEN".to_owned(), "a=b".to_owned()),
            ("DIR".to_owned(), "x".to_owned()),
        ];
        assert_eq!(product.env, env);
        assert_eq!(product.args, ["--flag", "KEY=arg"]);
        // A lowercase name is an argument (e.g. awk's `var=value`).
        assert_eq!(parse("t rawk exit fs=, x").unwrap()[0].args, ["fs=,", "x"]);
    }

    #[test]
    fn rejects_a_short_line_and_an_unknown_mode_with_the_line_number() {
        assert_eq!(
            parse("\nrush rush").unwrap_err().split(':').next(),
            Some("line 2")
        );
        assert!(
            parse("rush rush sometimes")
                .unwrap_err()
                .contains("unknown mode")
        );
    }
}

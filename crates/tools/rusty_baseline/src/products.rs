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

/// One product: a binary target of a workspace package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub package: String,
    /// Cargo features to enable, comma-separated as `--features` takes them.
    pub features: Option<String>,
    pub bin: String,
    pub mode: Mode,
    /// Environment variables set for the run, from leading `KEY=value`s.
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
}

/// Parses the product list: one product per line,
/// `<package>[:<features>] <bin> <exit|idle> [KEY=value...] [args...]`.
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

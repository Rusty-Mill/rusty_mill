//! `completion SHELL`: a completion script for bash, zsh, fish, Nushell or
//! PowerShell, generated from the catalog and the commands that are not
//! methods.
//!
//! Every script does the same: find the command being typed (the first
//! word after the global flags, or after the last `+`), then offer its
//! words. After a flag that takes a path it offers files; after any other
//! flag that takes a value it offers nothing, since the value is the
//! user's. Targets are not completed: that would need the server.

use std::fmt::Write as _;

use rusty_mux_proto::schema::{self, Ty};

use crate::cli::Shell;

/// Global flags, before the first command.
const GLOBAL: &[&str] = &["--socket", "--host", "--json"];

/// Flags whose value is a path.
const PATH_FLAGS: &[&str] = &["--socket", "--cwd", "--dir", "--file"];

/// The words of each command that is not a method: its verbs and flags.
/// `help` and `api` also take a method's name, added in [`commands`].
const COMMAND_WORDS: &[(&str, &[&str])] = &[
    ("events", &["--after", "--follow"]),
    ("do", &[]),
    ("config", &["check", "list"]),
    (
        "hosts",
        &[
            "list",
            "add",
            "remove",
            "pull",
            "push",
            "--socket",
            "--ssh",
            "--udp-host",
            "--replace",
            "--dry-run",
        ],
    ),
    ("token", &["create", "rotate", "revoke", "--file"]),
    (
        "ssh",
        &[
            "check",
            "diagnose",
            "setup",
            "remove",
            "--hostname",
            "--user",
            "--port",
            "--identity",
            "--write",
        ],
    ),
    ("terminfo", &["check", "setup", "remove", "--dry-run"]),
    ("watch", &["--after"]),
    (
        "wait",
        &["--block", "--window", "--session", "--event", "--timeout"],
    ),
    (
        "run",
        &[
            "--window",
            "--cols",
            "--rows",
            "--cwd",
            "--wait",
            "--timeout",
            "--",
        ],
    ),
    (
        "agent",
        &[
            "setup",
            "status",
            "remove",
            "skill",
            "--agent",
            "--project",
            "--dir",
            "--dry-run",
            "--name-session",
        ],
    ),
    ("bridge", &[]),
    ("own", &[]),
    ("server", &["stop"]),
    ("api", &["list", "describe", "llms"]),
    ("help", &[]),
    ("completion", &["bash", "zsh", "fish", "nu", "powershell"]),
];

/// Flags of the commands that are not methods and take no value.
const BARE_FLAGS: &[&str] = &[
    "--json",
    "--follow",
    "--wait",
    "--project",
    "--dry-run",
    "--name-session",
    "--",
];

/// What a script needs: each command with its words, the top-level words,
/// and the flags after which a value, not a word, comes next.
#[derive(Debug)]
struct Spec {
    top: Vec<String>,
    commands: Vec<(&'static str, Vec<String>)>,
    value_flags: Vec<String>,
}

fn spec() -> Spec {
    let methods = || schema::CATALOG.iter().map(|m| m.name.to_owned());
    let mut commands: Vec<(&'static str, Vec<String>)> = schema::CATALOG
        .iter()
        .map(|m| {
            (
                m.name,
                m.params.iter().map(|p| format!("--{}", p.name)).collect(),
            )
        })
        .collect();
    for (name, words) in COMMAND_WORDS {
        let mut words: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
        if matches!(*name, "help" | "api") {
            words.extend(methods());
        }
        commands.push((name, words));
    }
    let mut top: Vec<String> = GLOBAL.iter().map(|g| (*g).to_owned()).collect();
    top.extend(commands.iter().map(|(name, _)| (*name).to_owned()));
    let mut value_flags: Vec<String> = commands
        .iter()
        .flat_map(|(_, words)| words.iter())
        .chain(top.iter())
        .filter(|w| w.starts_with("--") && !BARE_FLAGS.contains(&w.as_str()))
        .filter(|w| !is_method_switch(w))
        .cloned()
        .collect();
    value_flags.sort_unstable();
    value_flags.dedup();
    Spec {
        top,
        commands,
        value_flags,
    }
}

/// Whether `flag` is a catalog switch: a parameter of type flag, given
/// without a value.
fn is_method_switch(flag: &str) -> bool {
    schema::CATALOG
        .iter()
        .flat_map(|m| m.params)
        .any(|p| p.ty == Ty::Flag && flag.strip_prefix("--") == Some(p.name))
}

/// The completion script for `shell`.
pub(crate) fn script(shell: Shell) -> String {
    let spec = spec();
    match shell {
        Shell::Bash => bash(&spec),
        Shell::Zsh => zsh(&spec),
        Shell::Fish => fish(&spec),
        Shell::Nu => nu(&spec),
        Shell::PowerShell => powershell(&spec),
    }
}

/// `words` single-quoted and space-separated; no word holds a quote.
fn quoted(words: &[String]) -> String {
    words
        .iter()
        .map(|w| format!("'{w}'"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn bash(spec: &Spec) -> String {
    let mut arms = String::new();
    for (name, words) in &spec.commands {
        let _ = writeln!(arms, "        {name}) words='{}' ;;", words.join(" "));
    }
    format!(
        r#"# rusty-mux-ctl completion for bash. Load it with:
#   source <(rusty-mux-ctl completion bash)
_rusty_mux_ctl() {{
    local cur=${{COMP_WORDS[COMP_CWORD]}} prev=${{COMP_WORDS[COMP_CWORD-1]}} cmd= words i w
    COMPREPLY=()
    for ((i = 1; i < COMP_CWORD; i++)); do
        case ${{COMP_WORDS[i]}} in
            --socket|--host) ((i++)) ;;
            --json) ;;
            +) cmd= ;;
            *) [[ -z $cmd ]] && cmd=${{COMP_WORDS[i]}} ;;
        esac
    done
    case $prev in
        {path})
            compopt -o filenames 2>/dev/null
            while IFS= read -r w; do COMPREPLY+=("$w"); done < <(compgen -f -- "$cur")
            return ;;
        {value}) return ;;
    esac
    case $cmd in
        '') words='{top}' ;;
{arms}        *) return ;;
    esac
    while IFS= read -r w; do COMPREPLY+=("$w"); done < <(compgen -W "$words" -- "$cur")
}}
complete -F _rusty_mux_ctl rusty-mux-ctl
"#,
        path = PATH_FLAGS.join("|"),
        value = spec.value_flags.join("|"),
        top = spec.top.join(" "),
    )
}

fn zsh(spec: &Spec) -> String {
    let mut arms = String::new();
    for (name, words) in &spec.commands {
        let _ = writeln!(arms, "        {name}) compadd -- {} ;;", quoted(words));
    }
    format!(
        r#"#compdef rusty-mux-ctl
# rusty-mux-ctl completion for zsh. Save it as _rusty-mux-ctl in a directory
# on $fpath, or load it after compinit with:
#   source <(rusty-mux-ctl completion zsh)
_rusty_mux_ctl() {{
    local cmd= prev=${{words[CURRENT-1]}} i
    for ((i = 2; i < CURRENT; i++)); do
        case ${{words[i]}} in
            --socket|--host) ((i++)) ;;
            --json) ;;
            +) cmd= ;;
            *) [[ -z $cmd ]] && cmd=${{words[i]}} ;;
        esac
    done
    case $prev in
        {path}) _files; return ;;
        {value}) return 1 ;;
    esac
    case $cmd in
        '') compadd -- {top} ;;
{arms}        *) return 1 ;;
    esac
}}
if [[ $funcstack[1] == _rusty-mux-ctl ]]; then
    _rusty_mux_ctl "$@"
else
    compdef _rusty_mux_ctl rusty-mux-ctl
fi
"#,
        path = PATH_FLAGS.join("|"),
        value = spec.value_flags.join("|"),
        top = quoted(&spec.top),
    )
}

fn fish(spec: &Spec) -> String {
    let mut arms = String::new();
    for (name, words) in &spec.commands {
        let _ = writeln!(
            arms,
            "        case {name}\n            printf '%s\\n' {}",
            quoted(words)
        );
    }
    format!(
        r#"# rusty-mux-ctl completion for fish. Save it as
# ~/.config/fish/completions/rusty-mux-ctl.fish, or load it with:
#   rusty-mux-ctl completion fish | source
function __rusty_mux_ctl_words
    set -l tokens (commandline -opc)
    set -l cmd ''
    set -l skip 0
    for t in $tokens[2..-1]
        if test $skip = 1
            set skip 0
            continue
        end
        switch $t
            case --socket --host
                set skip 1
            case --json
            case +
                set cmd ''
            case '*'
                test -z "$cmd"; and set cmd $t
        end
    end
    switch "$tokens[-1]"
        case {path}
            __fish_complete_path (commandline -ct)
            return
        case {value}
            return
    end
    switch "$cmd"
        case ''
            printf '%s\n' {top}
{arms}    end
end
complete -c rusty-mux-ctl -f -a '(__rusty_mux_ctl_words)'
"#,
        path = PATH_FLAGS.join(" "),
        value = spec.value_flags.join(" "),
        top = quoted(&spec.top),
    )
}

fn nu(spec: &Spec) -> String {
    let mut arms = String::new();
    for (name, words) in &spec.commands {
        let list = words
            .iter()
            .map(|w| format!("'{w}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(arms, "        '{name}' => [{list}]");
    }
    let list = |words: &[String]| {
        words
            .iter()
            .map(|w| format!("'{w}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let paths: Vec<String> = PATH_FLAGS.iter().map(|p| (*p).to_owned()).collect();
    format!(
        r#"# rusty-mux-ctl completion for Nushell. Save it and add to config.nu:
#   use /path/to/rusty-mux-ctl.nu *
def "nu-complete rusty-mux-ctl" [context: string] {{
    # Words as the shell splits them: a quoted value, closed or still being
    # typed, is one word. The first is the command; the last is the word
    # under the cursor unless the line ends in a space.
    let words = ($context | parse -r r#'(?<w>"[^"]*"?|'[^']*'?|`[^`]*`?|\S+)'# | get w | skip 1)
    let tokens = if ($context =~ '\s$') {{ $words }} else {{ $words | drop 1 }}
    mut cmd = ''
    mut skip = false
    for t in $tokens {{
        if $skip {{
            $skip = false
        }} else if $t in ['--socket', '--host'] {{
            $skip = true
        }} else if $t == '+' {{
            $cmd = ''
        }} else if $t != '--json' and $cmd == '' {{
            $cmd = $t
        }}
    }}
    let prev = ($tokens | last | default '')
    if $prev in [{paths}] {{
        return null
    }}
    if $prev in [{value}] {{
        return []
    }}
    match $cmd {{
        '' => [{top}]
{arms}        _ => []
    }}
}}

export extern "rusty-mux-ctl" [
    ...args: string@"nu-complete rusty-mux-ctl"
]
"#,
        paths = list(&paths),
        value = list(&spec.value_flags),
        top = list(&spec.top),
    )
}

fn powershell(spec: &Spec) -> String {
    let list = |words: &[String]| {
        words
            .iter()
            .map(|w| format!("'{w}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut arms = String::new();
    for (name, words) in &spec.commands {
        let _ = writeln!(arms, "        '{name}' {{ @({}) }}", list(words));
    }
    let paths: Vec<String> = PATH_FLAGS.iter().map(|p| (*p).to_owned()).collect();
    format!(
        r#"# rusty-mux-ctl completion for PowerShell. Add to your profile:
#   rusty-mux-ctl completion powershell | Out-String | Invoke-Expression
Register-ArgumentCompleter -Native -CommandName rusty-mux-ctl, rusty-mux-ctl.exe -ScriptBlock {{
    param($wordToComplete, $commandAst, $cursorPosition)
    $done = @($commandAst.CommandElements | Select-Object -Skip 1 |
        Where-Object {{ $_.Extent.EndOffset -lt $cursorPosition }} |
        ForEach-Object {{ $_.Extent.Text }})
    $cmd = ''
    $skip = $false
    foreach ($t in $done) {{
        if ($skip) {{ $skip = $false }}
        elseif ($t -cin @('--socket', '--host')) {{ $skip = $true }}
        elseif ($t -ceq '+') {{ $cmd = '' }}
        elseif ($t -cne '--json' -and $cmd -eq '') {{ $cmd = $t }}
    }}
    $prev = if ($done.Count) {{ $done[-1] }} else {{ '' }}
    if ($prev -cin @({paths}, {value})) {{ return }}
    $words = switch -CaseSensitive -Exact ($cmd) {{
        '' {{ @({top}) }}
{arms}        default {{ @() }}
    }}
    $words | Where-Object {{ $_.StartsWith($wordToComplete, [StringComparison]::Ordinal) }} |
        ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}
}}
"#,
        paths = list(&paths),
        value = list(&spec.value_flags),
        top = list(&spec.top),
    )
}

#[cfg(test)]
mod tests;

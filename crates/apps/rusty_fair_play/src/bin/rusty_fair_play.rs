//! `rusty_fair_play`: serve the Fair Play API and web UI.
//!
//! ```text
//! rusty_fair_play [--data-dir DIR] [--addr HOST:PORT] [--web-dir DIR] [--allow-remote] [--no-seed]
//! rusty_fair_play seed [--data-dir DIR] [--people FILE] [--splits FILE] [--cards FILE]
//! ```
//!
//! `--web-dir` serves the built web UI (`web/dist`) at `/`. The deck that
//! ships in the binary is loaded on first start unless `--no-seed`; `seed`
//! loads it (or `--cards`), plus people and splits files, and exits.
//!
//! `RUSTY_FAIR_PLAY_TOKEN` (16+ characters), when set, is required as a
//! bearer token on every `/api` request. It is read from the environment,
//! never an argument. Without it the server is open to whoever can reach
//! it, so it refuses a non-loopback address; `--allow-remote` needs the
//! token and something in front of it that terminates TLS.

use rusty_fair_play::api::Api;
use rusty_fair_play::server::Server;
use rusty_fair_play::service::Service;
use rusty_fair_play_domain::seed::{deck, parse_cards, parse_people, parse_splits, seed, SeedData};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

struct Options {
    data_dir: PathBuf,
    addr: SocketAddr,
    web_dir: Option<PathBuf>,
    allow_remote: bool,
    no_seed: bool,
    people: Option<PathBuf>,
    splits: Option<PathBuf>,
    cards: Option<PathBuf>,
    command: Vec<String>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut o = Options {
        data_dir: PathBuf::from("rusty_fair_play_data"),
        addr: SocketAddr::from(([127, 0, 0, 1], 8790)),
        web_dir: None,
        allow_remote: false,
        no_seed: false,
        people: None,
        splits: None,
        cards: None,
        command: Vec::new(),
    };
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--data-dir" => o.data_dir = value("--data-dir")?.into(),
            "--addr" => {
                let text = value("--addr")?;
                o.addr = text.parse().map_err(|e| format!("--addr {text:?}: {e}"))?;
            }
            "--web-dir" => o.web_dir = Some(value("--web-dir")?.into()),
            "--people" => o.people = Some(value("--people")?.into()),
            "--splits" => o.splits = Some(value("--splits")?.into()),
            "--cards" => o.cards = Some(value("--cards")?.into()),
            "--allow-remote" => o.allow_remote = true,
            "--no-seed" => o.no_seed = true,
            other if !other.starts_with('-') => o.command.push(other.to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(o)
}

fn run_seed(o: &Options) -> Result<String, String> {
    let data = SeedData {
        people: o
            .people
            .as_deref()
            .map(parse_people)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        deck: match &o.cards {
            Some(path) => parse_cards(path),
            None => deck(),
        }
        .map_err(|e| e.to_string())?,
        splits: o
            .splits
            .as_deref()
            .map(parse_splits)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        splits_label: o
            .splits
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    let r = seed(&o.data_dir, &data).map_err(|e| e.to_string())?;
    Ok(format!(
        "people {}+{} baselines {}+{} cards {}+{} splits {}+{} (created+existing)",
        r.people.created,
        r.people.existing,
        r.card_defaults.created,
        r.card_defaults.existing,
        r.cards.created,
        r.cards.existing,
        r.splits.created,
        r.splits.existing
    ))
}

fn run() -> Result<(), String> {
    let o = parse_args(std::env::args().skip(1))?;
    match o.command.as_slice() {
        [] => {}
        [word] if word == "seed" => {
            println!("{}", run_seed(&o)?);
            return Ok(());
        }
        [word, ..] => return Err(format!("unknown command {word:?}")),
    }
    let token = std::env::var("RUSTY_FAIR_PLAY_TOKEN").ok();
    if !o.addr.ip().is_loopback() {
        if token.is_none() {
            return Err(format!(
                "{} is not a loopback address; set RUSTY_FAIR_PLAY_TOKEN to serve there",
                o.addr
            ));
        }
        if !o.allow_remote {
            return Err(format!(
                "{} is not a loopback address; pass --allow-remote to serve plain HTTP there",
                o.addr
            ));
        }
    }
    let mut service =
        Service::open(&o.data_dir).map_err(|e| format!("opening {}: {e}", o.data_dir.display()))?;
    if !o.no_seed {
        let loaded = service
            .ensure_deck()
            .map_err(|e| format!("seeding the deck: {e}"))?;
        if let Some(r) = loaded {
            eprintln!(
                "rusty_fair_play: loaded the deck ({} cards)",
                r.cards.created
            );
        }
    }
    let api = Api::new(service, token)?;
    let mut server = Server::bind(o.addr, api).map_err(|e| format!("binding {}: {e}", o.addr))?;
    if let Some(dir) = o.web_dir {
        if !dir.join("index.html").is_file() {
            return Err(format!(
                "{} has no index.html; run `npm run build` in web/",
                dir.display()
            ));
        }
        server = server.with_web_dir(dir);
    }
    let addr = server.local_addr().map_err(|e| e.to_string())?;
    eprintln!("rusty_fair_play: listening on http://{addr}");
    server.run().map_err(|e| format!("serving: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rusty_fair_play: {message}");
            ExitCode::FAILURE
        }
    }
}

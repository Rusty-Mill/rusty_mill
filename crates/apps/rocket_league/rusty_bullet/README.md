# rusty_bullet

A from-scratch Rust reimplementation of Rocket League's client/server
physics and netcode architecture. Two reasons this exists: (1) the author
hits persistent online lag Psyonix hasn't fixed, and the fastest way to find
out where it actually comes from is to rebuild the architecture rather than
guess at it; (2) if the official servers ever shut down, this keeps
online-style play with friends possible. It does not use, copy, or decompile
Psyonix's code or assets — see [ARCHITECTURE.md](./ARCHITECTURE.md#non-goals)
for that boundary.

Architecture and physics fidelity are anchored on Psyonix's own public
disclosure: Jared Cone's GDC 2018 talk ["It IS Rocket Science! The Physics
and Networking of Rocket
League"](https://www.youtube.com/watch?v=ueEmiDM94IE)
([slides](https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf)).

## Status

Bootstrap — no simulation code yet. Phase 0 (the verification pipeline that
every later phase scores against) is next; see
[docs/roadmap/ROADMAP.md](./docs/roadmap/ROADMAP.md). Owner: baileyrd.

## Getting started

```bash
cargo build --workspace
cargo run -p rb_verify_cli --bin rb-verify -- <replay-file> <capture-file>
```

The ingestion adapters are stubs until Phase 0 delivery work lands their
real parsing backends (`boxcars` for replays, a BakkesMod capture format for
offline input+physics ground truth) — the CLI above currently reports that
rather than a score.

## Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries and data flow, and
[docs/architecture/SYSTEM-ARCHITECTURE.md](./docs/architecture/SYSTEM-ARCHITECTURE.md)
for the full systems-engineering treatment (context, trust boundaries,
principles). Key decisions are recorded in [docs/adr/](./docs/adr/); the
phased plan is in [docs/roadmap/ROADMAP.md](./docs/roadmap/ROADMAP.md);
open research questions are tracked in
[docs/research/RESEARCH-BACKLOG.md](./docs/research/RESEARCH-BACKLOG.md).

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security

See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License

Dual-licensed under [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE),
at your option. This project is an independent reimplementation and is not
affiliated with, endorsed by, or associated with Psyonix or Epic Games;
"Rocket League" is their trademark, referenced here only to describe
compatibility intent.

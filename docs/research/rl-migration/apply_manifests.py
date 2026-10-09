import re, pathlib, sys
root = pathlib.Path(sys.argv[1]); fam = root/"crates/apps/rocket_league/crates"
LAYER = '\n[package.metadata.rusty_mill]\nlayer = "apps"\n'
LINTS = '[lints.rust]\nunsafe_code = "deny"\n\n[lints.clippy]\nunwrap_used = "warn"\nexpect_used = "warn"\npanic = "warn"\n'
REPO = 'repository = "https://github.com/Rusty-Mill/rusty_mill"'
RENAME = {"..": "../replay-analyzer", "../scoring": "../replay-scoring", "../skills": "../replay-skills",
          "../value": "../replay-value", "../viewer": "../replay-viewer", "../pacifist": "../replay-pacifist"}
members = []
for d in sorted(p for p in fam.iterdir() if p.is_dir()):
    m = d/"Cargo.toml"; t = m.read_text()
    if "metadata.rusty_mill" in t: continue  # already wired by an earlier import
    members.append(f"crates/apps/rocket_league/crates/{d.name}")
    if d.name.startswith("rb_"):
        t = t.replace("repository.workspace = true", REPO)
        t = re.sub(r"\[lints\]\s*workspace = true\n?", LINTS, t)
    else:
        t = re.sub(r"\[workspace\]\nmembers = \[.*?\]\n+", "", t, flags=re.S)
        t = re.sub(r"\n\[profile\.release\][^\[]*", "\n", t)
        t = re.sub(r'path = "(\.\.(?:/\w+)?)"', lambda mo: f'path = "{RENAME.get(mo.group(1), mo.group(1))}"', t)
        t = re.sub(r'\{ git = "https://github.com/Rusty-Mill/rusty_mill", rev = "[0-9a-f]+"(, optional = true)? \}',
                   lambda mo: '{ workspace = true%s }' % (mo.group(1) or ""), t)
        t = t.replace('edition = "2021"\n', 'edition = "2021"\nlicense = "MIT OR Apache-2.0"\npublish = false\n', 1) if "license" not in t else t
    m.write_text(t.rstrip("\n") + "\n" + LAYER)
r = root/"Cargo.toml"; t = r.read_text()
t = t.replace('members = [\n', 'members = [\n' + "".join(f'    "{x}",\n' for x in members), 1)
t = t.replace('exclude = [\n', 'exclude = [\n    "crates/apps/rocket_league/rusty_bullet/tools/rb_tape_bot",\n', 1)
r.write_text(t); print(len(members), "members added")

#!/usr/bin/env bash
# Dry-run history rewrite for the Rocket League family import (see ../ROCKET-LEAGUE-APPS-MIGRATION-PLAN.md).
# Usage: filter.sh <rusty_bullet-checkout> <RLEvalSystem-checkout> <outdir>
# Sources are only cloned from, never modified. Outputs: $out/{rb,rl,rlpriv}, ready to `git fetch` and merge.
set -euo pipefail
RB=$1 RL=$2 OUT=$3; F=crates/apps/rocket_league
export GIT_LFS_SKIP_SMUDGE=1
rm -rf "$OUT"; mkdir -p "$OUT"; cd "$OUT"
git clone -q --no-local "$RB" rb; git clone -q --no-local "$RL" rl; git clone -q --no-local "$RL" rlpriv

# Private set: stays out of the public monorepo, goes to the private repo with history.
PRIV=(assets/myReplays assets/corpus assets/modes assets/case-studies data spire_capture docs/competitor
  docs/ballchasing-analyzer-teardown.md docs/ballchasing-comparison.md docs/ballchasing-dashboard-spec.md
  docs/ballchasing-parity.md docs/pacifist-system-teardown.md docs/viewer-vs-ballchasing-video.md
  docs/case-study-player-progression.md)
JUNK=(canonical-match.json replay-analyzer-main.rs replay-analyzer-Cargo.toml .serena Cargo.lock .github .gitignore .gitattributes)
sel() { for p in "$@"; do printf -- '--path\0%s\0' "$p"; done; }

( cd rb && git filter-repo --force --quiet \
  --path crates --path docs --path bakkesmod-plugin --path tools \
  --path README.md --path CHANGELOG.md --path RELEASE_NOTES.md --path AGENTS.md --path ARCHITECTURE.md --path WORKFLOW.md \
  --path THIRD_PARTY_NOTICES.md --path hit_tick_test.jsonl --path LICENSE-APACHE --path LICENSE-MIT \
  --path-rename crates/:$F/crates/ --path-rename docs/:$F/rusty_bullet/docs/ \
  --path-rename bakkesmod-plugin/:$F/rusty_bullet/bakkesmod-plugin/ --path-rename tools/:$F/rusty_bullet/tools/ \
  $(for f in README.md CHANGELOG.md RELEASE_NOTES.md AGENTS.md ARCHITECTURE.md WORKFLOW.md THIRD_PARTY_NOTICES.md hit_tick_test.jsonl; do echo --path-rename $f:$F/rusty_bullet/$f; done) \
  --path-rename LICENSE-APACHE:$F/LICENSE-APACHE --path-rename LICENSE-MIT:$F/LICENSE-MIT )

( cd rlpriv && sel "${PRIV[@]}" | xargs -0 git filter-repo --force --quiet )
( cd rl && sel "${PRIV[@]}" "${JUNK[@]}" | xargs -0 git filter-repo --force --quiet --invert-paths )
( cd rl && git filter-repo --force --quiet \
  --path-rename src/:$F/crates/replay-analyzer/src/ --path-rename tests/:$F/crates/replay-analyzer/tests/ \
  --path-rename build.rs:$F/crates/replay-analyzer/build.rs --path-rename Cargo.toml:$F/crates/replay-analyzer/Cargo.toml \
  --path-rename scoring/:$F/crates/replay-scoring/ --path-rename skills/:$F/crates/replay-skills/ \
  --path-rename value/:$F/crates/replay-value/ --path-rename viewer/:$F/crates/replay-viewer/ \
  --path-rename pacifist/:$F/crates/replay-pacifist/ --path-rename bc-clone/:$F/crates/bc-clone/ \
  --path-rename recon-check/:$F/crates/recon-check/ --path-rename app/:$F/crates/rleval-app/ \
  --path-rename service/:$F/rleval/service/ --path-rename scripts/:$F/rleval/scripts/ --path-rename docs/:$F/rleval/docs/ \
  --path-rename assets/replays/:$F/rleval/assets/replays/ \
  $(for f in Dockerfile docker-compose.yml .dockerignore .env.example README.md; do echo --path-rename $f:$F/rleval/$f; done) )

for r in rb rl rlpriv; do echo "$r: $(git -C $r rev-list --count HEAD) commits, pack $(git -C $r count-objects -vH | awk '/size-pack/{print $2,$3}')"; done

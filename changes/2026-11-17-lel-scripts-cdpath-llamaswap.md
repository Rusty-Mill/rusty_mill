---
category: Fixed
changelog: Let Em Loose smoke scripts ignore CDPATH and the rerun wrapper requires and passes LLAMA_SWAP before it does anything.
---
## 2026-11-17 - Smoke script wrapper fixes (CDPATH, LLAMA_SWAP)

- **Fixed:** with `CDPATH` set, `$(cd "$(dirname "$0")" && pwd)` printed the directory twice (`cd` prints it when it resolves through `CDPATH`), so the script directory was two lines and the `exec` that followed broke. The scripts now `unset CDPATH` and use `cd --`.
- **Fixed:** `1166-smoke-rerun.sh` did not check or pass `LLAMA_SWAP`: a plain (non-exported) shell variable never reached `1166-smoke-run.sh`, which failed late with `LLAMA_SWAP: set to the llama-swap host:port` after the guards had run. The wrapper now stops at once with the message and exports the value.
- **Known limitations:** shell scripts, tested with a stub run script and a stub `CDPATH`; not run on BaileyAi.

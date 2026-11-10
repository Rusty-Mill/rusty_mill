---
category: Added
changelog: rusty_conhost profile after the VT bulk path, with best-of-7 times on the eight 4 MB workloads and one proposed fix (card #785).
---
## 2026-11-10 - Console host profile after the VT bulk path

- **Added:** `docs/projects/term/conhost/profile-2026-11-10.md`, a profile of our console host on Beast at `db325fada` (card #785).
  - **Times:** best of 7 through `conhost_bench`'s console writer, with our host at 0.071x ConPTY's time on geomean.
  - **Where time goes:** the engine thread is busy 75-94% of a run. Parsing the mirror Grid takes 32-89% of its samples and `client_vt`'s escape scan up to 35%. The top stacks of both are listed.
  - **Proposed fix:** parse the mirror after a write's reply is submitted instead of before, so the parse overlaps the client's next write. Expected to save about 15% of wall time.
- **Known limitations:** documentation only; nothing in the host changed. The sampler was a throwaway tool kept outside the repo, and sampling slowed the host by about a third, so the profile shares are of samples, not of wall time.

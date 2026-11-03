---
category: Fixed
changelog: BBP's plan gate refuses a co-edited spec version that is not the head, so approval never drops concurrent edits.
---
## 2026-10-26 - BBP plan gate binds the head of a co-edited spec

- **Fixed:** a plan-gate request naming a co-edited spec version that is not the latest version of its spec is refused with `stale_subject`. Before, the gate could open on an older version and the approval silently dropped every edit accepted after it.
- **Known limitations:** no snapshot hash of the approved text yet; binding is by immutable version id.

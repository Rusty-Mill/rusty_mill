---
category: Added
changelog: The alien lens's hunt pipeline over the model port and a probe-runner port (#708).
---
## 2026-11-04 - Alien lens hunt pipeline

- **Added:** `orch_lens::call::ask_raw` asks a raw question over a list of providers, metered exactly as a job's call: reserve, save, ask, settle, save. A provider is passed over when it isn't configured, is cleared below the question's data class, is too small for the text, isn't built, is stopped by the experiment ceiling, or is unavailable. The settled cost and tokens come back with the answer. A failed call carries its measured tokens when the provider reported them (a reply refused for going over the output cap still used them) and `None` when usage is unknown. Job calls and raw calls now share one metering core.
- **Added:** `orch_lens::hunt::hunt` runs one hunt (design D75 §3). It makes one enumerate call, ranks the suspects, makes at most 3 hunt calls, and runs each probe on the candidate and on base through a `Probes` port, then classifies the result. It returns a `HuntRow` and one `ProbeRow` per probe that ran, both labelled with the task the calls were billed to. A probe that did not run gets no row (D78). Once the enumerate call has been made the hunt is recorded, even if a later step fails or a ceiling stops it.
- **Known limitations:** nothing calls the pipeline yet, and `Probes` has only a test fake. The `--alien` runner pass and the probe sandbox come in later slices; where probes run is waiting on an owner decision.

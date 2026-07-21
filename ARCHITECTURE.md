# Architecture

## Overview
`rusty_http` is intended to be an HTTP server/library in Rust. No code has
landed yet — this document records the intended shape so early
implementation follows it rather than drifting.

## Boundaries
<!-- Domain logic vs. I/O and framework details (ports-and-adapters).
     List the ports (interfaces) and the adapters that implement them. -->

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
|      |            |       |

## Structure
Modular monolith by default. Domain logic (request/response handling, routing)
should stay free of I/O and framework details (ports-and-adapters); a
component only gets split into its own service for a concrete forcing
function — independent scaling, a team/language boundary, or hard fault
isolation — none of which apply yet.

## Data flow
<!-- Diagram or short walkthrough of a request/event through the system -->

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
Not yet scoped — revisit once the first implementation exists.

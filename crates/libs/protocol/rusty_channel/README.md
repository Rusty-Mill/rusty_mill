# rusty_channel

Chat channels for AG-UI agents. A channel turns a message from a chat
service into a run of an [AG-UI](https://docs.ag-ui.com) agent and the
run's reply into a message back. The crate holds the mapping and nothing
else; the runner that owns the sockets supplies the I/O, so every rule is
tested without a network.

Step 5 of the [ADR-0007 follow-ons](../../../../docs/dev_phases/ADR_0007/FOLLOW-ONS.md).
Slack and Teams are the adapters so far; SMS follows the same shape.

## Shape

| Piece | What it does |
| --- | --- |
| `Channel` | Sans-IO: `receive(headers, body, now)` authenticates one inbound request and says what it was (`Challenge`, `Message`, `Ignored`); `reply(to, text)` describes the outbound `POST` that answers it. |
| `Thread` | One conversation's history. `run(&inbound)` appends the person's message and builds the `RunAgentInput` (the whole thread, no tools, no context); `absorb(events)` verifies and folds the run's events in and returns a `Reply` (the new assistant text, and the error if the run ended badly). |
| `slack::Slack` | The Events API: signature verification (`v0=` HMAC-SHA256 over `v0:<timestamp>:<body>`, five-minute skew), `url_verification`, `app_mention` anywhere and `message` in a direct message, retries and bots ignored; `chat.postMessage` back into the same thread. |
| `teams::Teams` | The Azure Bot Framework: the inbound bearer JWT verified (`RS256` against the framework's JWK Set, issuer, audience = app id, `exp`/`nbf` with five minutes of leeway, and the `serviceurl` claim against the activity's `serviceUrl`), `message` activities only, `<at>` mentions stripped; a reply activity posted to the conversation with a client-credentials token the runner fetches through `Channel::credential`. |
| `bot::Bot` (feature `bot`) | The runner: `rusty_serve` in, `rusty_agui::HttpAgent` to the agent, `rusty_tls` out. Answers the request first, runs on its own thread, fetches a credential when the channel asks. |

A mention starts (or continues) a Slack thread and the reply goes into it,
so one Slack thread is one AG-UI thread. A direct message is one AG-UI
thread per person. A Teams conversation id already names the thread (a
channel reply carries `;messageid=`), so it is the key as is.

Two hooks on `Channel` have defaults a static-credential service never
touches: `credential(now)` says whether the runner must fetch something
before replying (Teams' hourly token), and `reply_accepted(status, body)`
says whether the service took the reply (Slack answers `200` with
`"ok": false` on failure).

## Running a bot

`bot::Bot` (feature `bot`) is the runner: the service posts to the bot's
path, each message becomes a run of the agent at `AGENT_URL` (an agent
directly, or `rusty_agent_gateway`'s `agui` route), and the text goes
back over `rusty_tls`. Two examples wrap it:

```sh
SLACK_SIGNING_SECRET=… SLACK_BOT_TOKEN=xoxb-… AGENT_URL=http://127.0.0.1:8080/api/agent \
  cargo run -p rusty_channel --features bot --example slack_bot

TEAMS_APP_ID=… TEAMS_APP_PASSWORD=… AGENT_URL=http://127.0.0.1:8080/api/agent \
  cargo run -p rusty_channel --features bot --example teams_bot
```

Optional for both: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (a
bearer token for the agent). In the Slack app: a bot token with
`chat:write`, the Events API request URL pointed at `/slack/events`
through a public tunnel, and subscriptions to `app_mention` and
`message.im`. In Azure: a Bot resource with the Teams channel enabled and
its messaging endpoint pointed at `/teams/messages`; the framework's
signing keys are fetched once at start, so restart the bot when they
rotate.

Slack wants a `200` within three seconds and an agent can take longer, so
the request is answered first and the run happens on its own thread.
Threads live in memory for as long as the process does; a store is a
later step.

## Dependencies

First-party only: `rusty_agui` (types, verifier, reducer), `rusty_json`,
`rusty_uuid` (ids), `rusty_oauth` (HMAC-SHA256 and a constant-time
compare for Slack; JWK Sets, `RS256` verification and form encoding for
Teams). The `bot` feature adds `rusty_serve`, `rusty_http`, `rusty_tls`
and `rusty_agui`'s client for the runner and the examples.

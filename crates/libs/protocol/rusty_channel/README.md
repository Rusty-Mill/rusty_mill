# rusty_channel

Chat channels for AG-UI agents. A channel turns a message from a chat
service into a run of an [AG-UI](https://docs.ag-ui.com) agent and the
run's reply into a message back. The crate holds the mapping and nothing
else; the runner that owns the sockets supplies the I/O, so every rule is
tested without a network.

Step 5 of the [ADR-0007 follow-ons](../../../../docs/dev_phases/ADR_0007/FOLLOW-ONS.md).
Slack is the first adapter. Teams and SMS follow the same shape.

## Shape

| Piece | What it does |
| --- | --- |
| `Channel` | Sans-IO: `receive(headers, body, now)` authenticates one inbound request and says what it was (`Challenge`, `Message`, `Ignored`); `reply(to, text)` describes the outbound `POST` that answers it. |
| `Thread` | One conversation's history. `run(&inbound)` appends the person's message and builds the `RunAgentInput` (the whole thread, no tools, no context); `absorb(events)` verifies and folds the run's events in and returns a `Reply` (the new assistant text, and the error if the run ended badly). |
| `slack::Slack` | The Events API: signature verification (`v0=` HMAC-SHA256 over `v0:<timestamp>:<body>`, five-minute skew), `url_verification`, `app_mention` anywhere and `message` in a direct message, retries and bots ignored; `chat.postMessage` back into the same thread. |

A mention starts (or continues) a Slack thread and the reply goes into it,
so one Slack thread is one AG-UI thread. A direct message is one AG-UI
thread per person.

## Running the bot

`examples/slack_bot.rs` is a runner on `rusty_serve`: Slack posts to
`/slack/events`, each message becomes a run of the agent at `AGENT_URL`
(an agent directly, or `rusty_agent_gateway`'s `agui` route), and the
text goes back to Slack over `rusty_tls`.

```sh
SLACK_SIGNING_SECRET=… SLACK_BOT_TOKEN=xoxb-… AGENT_URL=http://127.0.0.1:8080/api/agent \
  cargo run -p rusty_channel --features bot --example slack_bot
```

Optional: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (a bearer token
for the agent). In the Slack app: a bot token with `chat:write`, the
Events API request URL pointed at `/slack/events` through a public tunnel,
and subscriptions to `app_mention` and `message.im`.

Slack wants a `200` within three seconds and an agent can take longer, so
the request is answered first and the run happens on its own thread.
Threads live in memory for as long as the process does; a store is a
later step.

## Dependencies

First-party only: `rusty_agui` (types, verifier, reducer), `rusty_json`,
`rusty_uuid` (ids), `rusty_oauth` (HMAC-SHA256 and a constant-time
compare). The `bot` feature adds `rusty_serve`, `rusty_http`, `rusty_tls`
and `rusty_agui`'s client for the example only.

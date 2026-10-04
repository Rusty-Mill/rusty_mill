# rusty_fair_play

A self-hosted front end for Eve Rodsky's **Fair Play** household cards,
built on [`rusty_fair_play_domain`](../../libs/storage/rusty_fair_play_domain)
(the domain `rusty_multimodal_db` ADR-0137 built, as its own libs crate).
A JSON HTTP API over the embedded stacks plus a web UI in [`web/`](web/).

## Run

```sh
cargo run -p rusty_fair_play -- [--data-dir rusty_fair_play_data] [--addr 127.0.0.1:8790] [--web-dir web/dist]
```

The deck that ships in the binary (the 100 original cards) is loaded on
first start; `--no-seed` skips that. Add people and splits from CSV with:

```sh
cargo run -p rusty_fair_play -- seed --data-dir DIR [--people people.csv] [--splits splits.csv] [--cards cards.csv]
```

(`people.csv` is `name` per row; `splits.csv` is
`parent_path,name,owner_name,minimum_standard_of_care`, e.g.
`2/Floors,Mopping,Ada,Mopped Sundays`; see the domain crate's `data/`.)
Loading is idempotent and never overwrites a card the family has edited.

The server speaks plain HTTP. Without a token it serves loopback only.
Set `RUSTY_FAIR_PLAY_TOKEN` (16+ characters) to require
`Authorization: Bearer <token>` on `/api`, and pass `--allow-remote` to
bind elsewhere (put TLS in front of it). The data directory is locked
while it is served.

## API

JSON, camelCase, ids are UUID strings. Errors are
`{"error":{"code","message"}}`: 400 malformed request, 401, 404,
409 conflict, 422 invalid value, 500. Inputs reject unknown fields.

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | no auth |
| GET | `/api/v1/snapshot` | `{"people":[Person],"cards":[Card]}` — one boot read |
| POST | `/api/v1/seed` | load the embedded deck; `{"cardDefaults":{created,existing},"cards":{…}}` |
| GET, POST | `/api/v1/people` | POST `{"name"}` → 201 Person; 409 if the name exists |
| GET, PATCH | `/api/v1/people/{id}` | PATCH `{"name"}` |
| GET, PATCH | `/api/v1/cards/{id}` | PATCH any of `name`, `suit`, `conception`, `planning`, `execution`, `minimumStandardOfCare`, `notes`, `ownerId`, `parentCardId`, `position`; absent = keep, `null` clears `ownerId`/`parentCardId` |
| POST | `/api/v1/cards` | `{"id"?,"name","suit","parentCardId"?,"ownerId"?,"conception"?,"planning"?,"execution"?,"minimumStandardOfCare"?,"notes"?}` → 201 Card |
| POST | `/api/v1/cards/{id}/split` | `{"children":[{"id"?,"name","ownerId"?,…}],"ownerId"?,"notes"?}` → 201 `{"parent":Card,"children":[Card]}` |
| POST | `/api/v1/cards/{id}/reset` | the six text fields back to the baseline; owner, parent, position and notes kept |
| GET | `/api/v1/cards/{id}/baseline` | `{"baseline":Baseline|null,"diff":[{"field","card","baseline"}]}` |
| PUT | `/api/v1/cards/{id}/position` | `{"position": n}` → 204 |

```ts
interface Person { id: string; name: string; player: number }
interface Card {
  id: string
  number: number | null            // 1..100 for a deck card
  name: string
  suit: 'Home' | 'Out' | 'Caregiving' | 'Magic' | 'Wild' | 'Unicorn Space'
  parentCardId: string | null      // the tree: a split-off card's parent
  position: number                 // sibling order under parentCardId
  ownerId: string | null           // null = unassigned
  conception: string; planning: string; execution: string
  minimumStandardOfCare: string[]
  notes: string
  origin: 'deck' | 'family'
  baselineId: string | null
  state: 'original' | 'edited' | 'custom'   // derived against the baseline, never stored
}
interface Baseline { id; number; name; suit; conception; planning; execution; minimumStandardOfCare }
```

The rules behind the API (ADR-0137): whoever holds a card owns all of its
Conception, Planning and Execution; a card can be split into cards with
different owners; the parent's owner holds what is left at that level;
the real "still undealt" list is the unowned **leaf** cards; editing any
of the six text fields makes a deck card `edited`, while owner, parent,
position and notes never do. There is no delete.

Refusals from the domain — a card made its own ancestor, an unknown
owner, a blank name, a reset on a custom card — are 422.

## Layout

`api` (pure router) and `dto` (wire shapes) sit over `service` (rules,
the three stacks), which sits over `rusty_fair_play_domain`.
`server` is the only file that touches a socket; `static_files` serves
`web/dist`. Both are `rusty_tick`'s, adapted.

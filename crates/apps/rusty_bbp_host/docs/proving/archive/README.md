# Archived proving runs

Records of runs made before log schema 2 (spec v0.5, roles as data). They describe the v0.4.1 protocol with its four fixed roles.

| Record | Schema | Outcome |
| --- | --- | --- |
| [`2026-10-09-slug-1.md`](2026-10-09-slug-1.md) | 1 | escalated on the iteration budget (environment fault) |

## Archiving a schema-1 data directory

A schema-1 log has no `schema` field and no roster. `FsStore::open` refuses the whole data directory when it finds one, naming the file ("log schema 1 or older is not supported ... archive it"). The raw store of the first run lived outside this repository (`attempt-N/` directories on the machine that ran it); none is committed here. To keep one:

```sh
mkdir -p ARCHIVE && mv DATA/tasks/*.log ARCHIVE/   # DATA is the old --dir
```

Then open new tasks in a fresh or cleaned `DATA`. Nothing reads `ARCHIVE`; the logs are evidence, not input.

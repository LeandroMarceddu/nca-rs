# Fixtures

Fixture folders live under `fixtures/`:

```text
fixtures/
  synthetic/<name>/{source.nc,config.toml,expected.json,NOTES.md}
  real/<name>/{source.nc,config.toml,expected.json,NOTES.md}
```

Rules:

- `source.nc` is the complete program; never truncate it silently.
- `config.toml` uses the schema in `docs/INTERFACES.md`. Omit tables you do not
  need; an omitted `[stock]` means inferred stock.
- `expected.json` is the full golden report, reviewed by the orchestrator before
  it is committed.
- `NOTES.md` states what the fixture proves, which post/machine produced it, and
  the permission for any real file.

See `docs/FIXTURES.md` for the required synthetic coverage and the review
process.

# Contributing

Issues and pull requests are welcome. A person reads every change before it
lands, and a change that alters a calculated number needs to show its working.

## Data boundary

This repository must never carry real data. No client, taxpayer, employee or
payroll records, no credentials, no tokens, no organisation names from a live
engagement. Everything under `examples/` is invented, and every figure in it is
round and obviously fictional for that reason.

The `.gitignore` blocks the file names real ledgers and payroll exports arrive
under. It is a safety net, not permission to try. If you need a new fixture,
write one by hand.

## Rates, thresholds and deadlines

Every statutory figure lives in `examples/drivers.csv` as data. None of them is
written into rule text, so changing a rate never means editing a rule.

If you change a rate, a threshold or an effective date, cite the primary source
in the pull request: the Australian Taxation Office page, the Revenue NSW page,
or the legislation itself. Update the table in `docs/model-assumptions.md` in
the same change, including the date you retrieved it. A secondary source, a blog
or an accounting firm summary is not enough on its own.

## Tests

The suite asserts calculated numbers, not just that the model loads. A test that
pins a figure must compute the expected value from first principles in the test
body, so a reviewer can follow the arithmetic without trusting the engine.

Run that test against the old code first. If it passes there too, it is not
testing your change.

```bash
uv run --locked --extra dev pytest -q
```

The same command CI runs. The packaging job additionally builds the wheel,
installs it into a clean environment and runs the real command line against the
real model tree, because a wheel that cannot find its own model is a broken
build artefact.

## Model changes

The model source under `model/` follows the layout IBM's own Git integration
writes: a `tm1project.json` manifest, `dimensions/`, `cubes/` with plain text
`.rules` beside the JSON, and `processes/` with plain text `.ti` beside the JSON.

Two rules hold for any change there:

1. Every file in the tree must appear in the manifest, directly or through a
   link from an object the manifest lists. The validator reports anything else,
   because a deployment would leave it behind.
2. Any cube with `SKIPCHECK` and rules needs a feeder pointing into it, from
   that cube or from another. Unfed calculated cells are invisible in a real
   database even though the rule is correct.

Run `pacioliscube validate` before you open the pull request.

## Pull requests

Keep a pull request to one change. Describe what it does and, where it touches a
number, show the command output rather than saying the tests pass.

Prose in this repository is Australian English and carries no em dashes.

## Security

Report a vulnerability privately through the process in
[SECURITY.md](SECURITY.md), not in a public issue.

# Contributing

Issues and pull requests are welcome. A person reads every change before it
lands, and a change that alters a calculated number needs to show its working.

## Data boundary

This repository must never carry real data. No client, taxpayer, employee or
payroll records, no credentials, no tokens, no organisation names from a live
engagement. Business data under `examples/` is invented. Statutory rates and
thresholds use the dated primary sources in `docs/model-assumptions.md`.

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
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The same commands CI runs: formatting and clippy in the `lint` job, then the
tests on Ubuntu and Windows, with line coverage measured on Ubuntu by
cargo-llvm-cov and a 90% floor. Install the git hooks once with
`pre-commit install` and `pre-commit install --hook-type commit-msg`; they run
the same commands. The packaging job additionally installs the release binary
and runs the real command line against the real model tree, because a binary
that cannot read its own model is a broken build artefact.

Where the engine reproduces Python's behaviour (decimal arithmetic, the JSON
reader, string case rules, paths, CSV reading and argument parsing), the
expected results come from `tests/data/python_golden.json`, recorded from the
last Python release; see [CHANGELOG.md](CHANGELOG.md). The mutation pilot in
[docs/mutation-baseline.md](docs/mutation-baseline.md) records the Python
engine and is kept as history.

`src/pyunicode.rs` records CPython 3.15.0's Unicode 17.0.0 character predicates
and case mappings. Its generator, `tools/gen_pyunicode.py`, requires that exact
interpreter. Generate into a temporary file, format it with the pinned rustfmt
and compare it with the committed table before replacing the table. Python is
needed only to regenerate it; the Rust build and binary use the saved data.

## Model changes

The model source under `model/` follows IBM's TM1 source specification for a
model kept in Git: a `tm1project.json` manifest, `dimensions/`,
`cubes/` with plain text `.rules` beside the JSON and dimensions named as
`{"@id": "Dimensions('X')"}` references, and `processes/` with plain text `.ti`
beside the JSON. Each `.ti` file holds `#region Prolog`, `#region Metadata`,
`#region Data` and `#region Epilog` blocks in that order, each closed by
`#endregion`, and each process JSON sets `HasSecurityAccess` (`false` unless
the process changes security data).
`tests/model.rs` holds the model to that form, and the `tm1git-conformance`
CI job has tm1gitpy read it as well (`ci/tm1gitpy_model.py`).

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

## Release checks

The release caller names the component checks that must have succeeded for the
exact release commit on `main`. It advances the policy SHA, `required-checks`
and `actions: read` together. Skipped, missing, cancelled or failed checks block
publication, including component tests skipped by a path filter. An aggregate
gates job cannot replace those checks. Before tagging, choose a main-branch
commit with successful component CI; a successful run for an older commit is
not evidence for the release. Tags and publication still require explicit approval.

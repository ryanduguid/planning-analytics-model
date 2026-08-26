# PaciolisCube

```
+----------------------------------------------------------------------+
|                             PaciolisCube                             |
+----------------------------------------------------------------------+
|            IBM Planning Analytics budget model as source             |
+----------------------------------+-----------------------------------+
| DR  what it gives you            | CR  what it needs                 |
+----------------------------------+-----------------------------------+
| runs TM1 rules with no server    | PA model files in Git format      |
| validates model structure        | CSV data for cube loading         |
| prints a P and L report          | -                                 |
+----------------------------------+-----------------------------------+
```

[![tests](https://github.com/ryanduguid/PaciolisCube/actions/workflows/ci.yml/badge.svg)](https://github.com/ryanduguid/PaciolisCube/actions/workflows/ci.yml)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-5C2D91.svg?labelColor=04001F)](LICENSE)
[![python](https://img.shields.io/badge/python-3.10%2B-5C2D91.svg?labelColor=04001F)](https://www.python.org/)

An IBM Planning Analytics budgeting model published as source, with an offline
engine that computes it and a test suite that asserts the answers.

Planning Analytics models usually live inside a server. You can read a rule file
on GitHub, but you cannot run it, and nothing tells you whether a change to it
broke a number. This repository holds a complete driver based budget for a
fictional Australian mining services group in the layout IBM's own Git
integration writes, plus a Python engine that parses the rules and evaluates
them with no TM1 server anywhere. Continuous integration recomputes the whole
model on every push and fails if a figure moves.

## What is here

- `model/` is the model: a `tm1project.json` manifest, 13 dimensions, 5 cubes,
  4 rule files and 8 TurboIntegrator processes. Rules are plain `.rules` text
  beside the cube JSON, scripts are plain `.ti` text beside the process JSON,
  which is what the Planning Analytics Git integration reads and writes.
- `pacioliscube/` is the engine: a rule parser, a structural validator, a cell
  store with weighted consolidation, a CSV loader and a command line.
- `examples/` is invented input data. Every entity, rate and volume in it is
  fictional.
- `docs/model-assumptions.md` traces every statutory figure to its source, and
  says for each one whether it was read from the legislation or confirmed
  against the administering authority's own page.

## Install

```bash
pip install pacioliscube
```

The runtime imports nothing outside the Python standard library. From a clone:

```bash
uv run --locked --extra dev pytest -q
```

## Use

Check the model's structure:

```bash
pacioliscube validate model
```

```
0 errors, 0 warnings
```

Print a profit and loss from the shipped data:

```bash
pacioliscube report model --data examples --year FY2026-27 --version Budget
```

```
Profit and loss for FY2026-27, Budget
PnL at FY, Group, All Cost Centres

Revenue             52,764,000
Direct Costs      (14,884,800)
Gross Margin        37,879,200
Employment Costs  (11,428,418)
Overheads          (6,384,000)
EBITDA              20,066,782
Depreciation       (1,382,143)
EBIT                18,684,640
```

Read one cell, consolidated or leaf:

```bash
pacioliscube calculate model --data examples --cell "PnL:FY2026-27,Budget,FY,Group,All Cost Centres,EBIT,Amount"
```

Each report line is rounded to whole dollars on its own, so a subtotal can sit a
dollar away from the lines above it. `calculate` prints the unrounded figure.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Clean |
| 1 | A usage or input error, including a report that does not fit the model |
| 2 | The model does not load, or validation reports an error |
| 3 | A calculation failed |

## The model

Two entities, CivilCo (civil earthworks) and HaulCo (haulage), roll up to a
`Group`. The year ends 30 June, so `Period` runs July to June. FY2025-26 is held
as `Actual` and FY2026-27 as `Budget`.

Four cubes feed a fifth. `Drivers` holds statutory and planning rates and has no
rules at all. `Workforce` turns headcount and pay rates into base pay,
superannuation and payroll tax. `Revenue` turns billable hours and charge rates
into revenue, and litres into fuel cost. `Capex` spreads fleet additions over
each asset class life. All three feed `PnL`, which carries the statement from
revenue down to EBIT.

No rule file holds a statutory rate. Every one is read from the `Drivers` cube
with `DB()`, so changing the superannuation guarantee percentage means editing a
CSV, not a rule. A test fails the build if a number in any rule or script
carries a value the driver file ships.

## What the tests actually check

The suite computes the model and asserts figures, rather than only checking that
the source parses. Each expected number is written out longhand in the test, so
a reviewer can follow the arithmetic without trusting the engine:

- Monthly base pay is headcount times the annual rate over twelve.
- Superannuation is capped at the maximum contribution base, and the cap is
  proved to bite by asserting the gap from the uncapped figure.
- Each year caps at its own base, so correcting one year cannot pass silently.
- Payroll tax is levied on pay grossed up by superannuation, not on pay alone.
- The payroll tax threshold credit reaches the designated group employer and no
  other cost centre.
- Depreciation is annual additions over the asset life in months, and an asset
  class with no life set charges nothing rather than raising.
- EBIT equals EBITDA less depreciation at the Group, across the full year.
- The `PnL` wages line equals `Workforce` base pay summed over roles.

Breaking any of those in the model source turns the suite red.

## What it does not do

- It is not tax advice, and it is not a payroll or lodgement engine. The
  superannuation and payroll tax treatment here is a planning approximation.
- It is not a maintained rate table. The statutory figures are sample data
  pinned at the date in `docs/model-assumptions.md`, and they go stale.
- It is not a full set of accounts. No balance sheet, no cash flow, no income
  tax, no GST, no foreign currency, no intercompany elimination.
- The engine covers the subset of the TM1 rules language this model uses. It is
  not a Planning Analytics reimplementation, and it will not run an arbitrary
  model.

## Client data

No real data belongs in this repository. The `.gitignore` blocks the file names
ledgers and payroll exports arrive under, and everything in `examples/` is
invented. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Author

Written by Ryan Duguid, a provisional member of Chartered Accountants ANZ,
independently, in his own time and on his own equipment. Nothing here is the
work of any employer, and no client data was used to build or test it.

Parts of this repository were written with AI assistance. Every statutory figure
is traced to its source in `docs/model-assumptions.md`, which records what was
read from the legislation, what was taken from the administering authority's own
page, and who confirmed the figures that could not be fetched.

## Licence

MIT. See [LICENSE](LICENSE).

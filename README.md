<p align="center">
  <picture>
    <source media="(max-width: 600px) and (prefers-color-scheme: light)" srcset=".github/assets/banner-phone-light.svg">
    <source media="(max-width: 600px)" srcset=".github/assets/banner-phone.svg">
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/banner-light.svg">
    <img src=".github/assets/banner.svg" width="100%" alt="TM1 Model. A TM1 budget model as source, tested offline. Pixel art of a puzzle cube beside the TM1 Model wordmark in slab-serif block letters.">
  </picture>
</p>

Run and test an IBM Planning Analytics (TM1) budget model's rules offline, without a TM1 server.

Build the command line from a clone (rustup installs the pinned toolchain on first use), then print a first result from the synthetic model built into the binary:

```bash
git clone https://github.com/ryanduguid/planning-analytics-model.git
cd planning-analytics-model
cargo install --locked --path .
pacioliscube demo
```

```text
PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount = 1652100
```

```
+----------------------------------------------------------------------+
|                       planning-analytics-model                       |
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

The PaciolisCube budget model. This repository was named PaciolisCube until September 2026; the old URL redirects.

<div align="center">

[![tests](https://img.shields.io/github/actions/workflow/status/ryanduguid/planning-analytics-model/ci.yml?branch=main&label=tests&labelColor=000000&style=flat)](https://github.com/ryanduguid/planning-analytics-model/actions/workflows/ci.yml) [![Codecov](https://img.shields.io/codecov/c/github/ryanduguid/planning-analytics-model?label=coverage&labelColor=000000&style=flat)](https://codecov.io/gh/ryanduguid/planning-analytics-model) [![CodeQL](https://img.shields.io/github/actions/workflow/status/ryanduguid/planning-analytics-model/codeql.yml?branch=main&label=CodeQL&labelColor=000000&style=flat)](https://github.com/ryanduguid/planning-analytics-model/actions/workflows/codeql.yml) [![Snyk](https://img.shields.io/github/actions/workflow/status/ryanduguid/planning-analytics-model/snyk.yml?branch=main&label=Snyk&labelColor=000000&style=flat)](https://github.com/ryanduguid/planning-analytics-model/actions/workflows/snyk.yml) [![SonarCloud](https://img.shields.io/sonar/quality_gate/ryanduguid_planning-analytics-model?server=https%3A%2F%2Fsonarcloud.io&label=SonarCloud&labelColor=000000&style=flat)](https://sonarcloud.io/summary/new_code?id=ryanduguid_planning-analytics-model) [![Codacy code quality](https://img.shields.io/codacy/grade/85d3cab41a8e4cc5ac3dd47c8996c78e?label=Codacy&labelColor=000000&style=flat)](https://app.codacy.com/gh/ryanduguid/planning-analytics-model/dashboard)<br> [![OpenSSF Scorecard](https://img.shields.io/ossf-scorecard/github.com/ryanduguid/planning-analytics-model?label=Scorecard&labelColor=000000&style=flat)](https://scorecard.dev/viewer/?uri=github.com/ryanduguid/planning-analytics-model) [![OpenSSF Best Practices](https://img.shields.io/cii/level/15428?label=Best%20Practices&labelColor=000000&style=flat)](https://www.bestpractices.dev/projects/15428) [![SLSA 3](https://img.shields.io/badge/SLSA-level%203-990024?labelColor=000000&style=flat&color=990024)](https://github.com/ryanduguid/planning-analytics-model/attestations) [![VirusTotal](https://img.shields.io/badge/VirusTotal-scanned-990024?labelColor=000000&style=flat&logo=virustotal&logoColor=FFD32A&color=990024)](https://github.com/ryanduguid/planning-analytics-model/releases/latest)<br> [![rust: 1.99](https://img.shields.io/badge/rust-1.99-66023C?labelColor=000000&style=flat&color=66023C)](https://www.rust-lang.org/) [![licence: FSL-1.1-ALv2](https://img.shields.io/badge/licence-FSL--1.1--ALv2-66023C?labelColor=000000&style=flat&color=66023C)](LICENSE)

</div>

An IBM Planning Analytics budgeting model published as source, with an offline
engine that computes it and a test suite that asserts the answers.

**Package lifecycle:** the engine is a Rust command line, version 1.0.0,
built from source. It replaces the Python package `pacioliscube` 0.3.0;
its library API is not carried forward. See [CHANGELOG.md](CHANGELOG.md).

Planning Analytics models usually live inside a server. You can read a rule file
on GitHub, but you cannot run it, and nothing tells you whether a change to it
broke a number. This repository holds a complete driver based budget for a
fictional Australian mining services group in the source form of IBM's TM1
source specification, which the Planning Analytics Git integration reads, plus a
Rust engine that parses the rules and evaluates them with no TM1 server
anywhere. Continuous integration recomputes the whole model on every push and
fails if a figure moves, and it loads the model with
[tm1gitpy](https://github.com/KnowledgeSeed/tm1gitpy), an independent TM1 Git
implementation, to check that both readers see the same model.

On this page:

- [Install](#install)
- [Use](#use)
- [Use from an agent over MCP](#use-from-an-agent-over-mcp)
- [Exit codes](#exit-codes)
- [The model](#the-model)
- [What the tests actually check](#what-the-tests-actually-check)
- [What it does not do](#what-it-does-not-do)
- [Client data](#client-data)

## What is here

- `model/` is the model: a `tm1project.json` manifest, 13 dimensions, 5 cubes,
  4 rule files and 8 TurboIntegrator processes. Rules are plain `.rules` text
  beside the cube JSON, scripts are plain `.ti` text beside the process JSON
  with a `#region` block for each of the Prolog, Metadata, Data and Epilog
  procedures, and cubes name their dimensions as `Dimensions('X')`
  references, as the specification's examples show.
- `src/` is the engine: a rule parser, a structural validator, a cell store
  with weighted consolidation, a CSV loader and the `pacioliscube` command
  line. Figures are exact decimals, never floating point.
- `examples/` contains fictional entities, commercial rates and volumes, plus
  dated statutory rates and thresholds cited in `docs/model-assumptions.md`.
- `docs/model-assumptions.md` traces every statutory figure to its source, and
  says for each one whether it was read from the legislation or confirmed
  against the administering authority's own page.

## Install

Build the command line from a clone, which also gives you the model files and
the tests. The toolchain is pinned in `rust-toolchain.toml`; rustup installs it
on first use.

```bash
git clone https://github.com/ryanduguid/planning-analytics-model.git
cd planning-analytics-model
cargo install --locked --path .
```

The engine needs no network, server or runtime beyond the binary. From a
clone, run the tests with:

```bash
cargo test --locked
```

## Use

The installed binary carries a first result that works without a clone:

```bash
pacioliscube demo
```

```text
PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount = 1652100
```

It reads the synthetic model and five CSV inputs built into the binary, without
accessing a network. It writes them to a new private temporary directory, reads
them from there and removes that directory before it exits; nothing is written
where it runs. These are dated planning assumptions, not current statutory
advice or evidence of native TM1 agreement. This command is pending
publication; the published 0.3.0 route uses the source directories below.

Run these examples from the root of the source clone above; they use the
checked-in `model/` and `examples/` directories.

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

For another reporting tool or a script, add `--format json` or `--format csv`
to the same `report` command. Both write to standard output after the complete
report has been calculated. Decimal values remain exact strings, without
whole-dollar rounding or a conversion to floating point.

JSON uses schema `profit-and-loss.v1`: the cube name and ordered rows, each with
its account, resolved dimension coordinates, stored `value` and signed `effect`.
Costs have a negative effect; a credit cost has a positive effect. CSV carries
the cube and one column per applied dimension, followed by `value` and `effect`.
CSV labels that start with spreadsheet formula prefixes get a leading apostrophe;
JSON retains their exact text. Import CSV amount columns as text to preserve
precision in Excel, whose numeric cells retain only 15 significant digits.
The default `--format text` keeps the printed statement shown above.

That block is engine output. The independent arithmetic behind these figures is
in [native comparison](docs/native-comparison.md), which derives 8 cells from the
synthetic inputs without the engine, and in the calculation tests.

Explain the inputs and arithmetic behind a cell:

```bash
pacioliscube explain model --data examples --cell "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Contract Revenue,Amount"
```

`explain` prints JSON with the requested cells and a breakdown of their
dependencies. Each cell records its exact value and whether it came from an
input, an unwritten cell's default zero, a rule or a weighted consolidation.
Rules include their model-relative file and line, input cell references, and
the arithmetic and comparisons actually evaluated. Consolidations list each
child value, weight and contribution. Shared cells appear once; every weighted
path still contributes to its parent.

Amounts, operands and weights are decimal strings in JSON. Comparisons remain
booleans. An `IF` records the selected branch and evaluates only that branch.
The command validates the whole model, then calculates only the requested cells
and their dependencies. It does not check unrelated calculations. Repeat
`--cell` to explain several cells in one run; a failed request emits no partial
JSON. Large totals can have many dependencies, so start with a monthly cell.

This explains the offline model's calculations. It does not establish the
correctness of source inputs, accounting treatment or agreement with native TM1.

For changes between two input snapshots, use the
[scenario comparison command](docs/scenario-comparison.md).

The offline CSV loader refuses conflicting values for the same cell and names
both rows. Repeated rows with equal decimal values remain valid, including when
element names differ only in case or surrounding whitespace.

## Use from an agent over MCP

`pacioliscube mcp` serves the same engine to a Model Context Protocol client
over standard input and output, so an agent can query the model directly. It
needs no TM1 server and has no write path.

```bash
pacioliscube mcp /path/to/planning-analytics-model/model --data /path/to/planning-analytics-model/examples
```

| Tool | What it returns |
| --- | --- |
| `list_cubes` | Each cube's dimensions in coordinate order, rules file, `SKIPCHECK` and rule and feeder counts |
| `list_dimensions` | Elements of every dimension, or one, with their types and weighted children |
| `read_slice` | Exact values for every combination of the elements given per dimension, up to 10,000 cells |
| `explain_rules` | A cube's rule statements with file and line, and the feeders whose targets reach each rule's area |
| `explain_cells` | The `explain` JSON evidence for up to 16 cells |
| `render_report` | The profit and loss as `report` prints it, in `text`, `json` or `csv` |

The model and data directories are fixed when the server starts. No tool
takes a path: tools accept only cube, dimension and element names, cell
references and report options, so a client cannot point the engine at any
other file. Unknown fields, control characters, names over 256 characters and
oversized requests are refused before the model is read. Each call reads the
model files afresh, and `read_slice`, `explain_cells` and `render_report`
refuse a model with validation errors, as the command line does.

`explain_rules` reads feeder reach statically from the rules files, expanding
consolidated elements to their leaves. It is not TM1's feeder evaluation.

Add it to Claude Code (use absolute paths; `--scope project` writes the
entry to a shared `.mcp.json` instead):

```bash
claude mcp add pacioliscube -- pacioliscube mcp /path/to/planning-analytics-model/model --data /path/to/planning-analytics-model/examples
```

Add it to Codex:

```bash
codex mcp add pacioliscube -- pacioliscube mcp /path/to/planning-analytics-model/model --data /path/to/planning-analytics-model/examples
```

or in `~/.codex/config.toml`:

```toml
[mcp_servers.pacioliscube]
command = "pacioliscube"
args = ["mcp", "/path/to/planning-analytics-model/model", "--data", "/path/to/planning-analytics-model/examples"]
```

`mcp` is handled before the main argument parser, so `pacioliscube --help`
does not list it; `pacioliscube mcp --help` describes it.

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
each asset class life. All 3 feed `PnL`, which carries the statement from
revenue down to EBIT.

No rule file holds a statutory rate. Every one is read from the `Drivers` cube
with `DB()`, so changing the superannuation guarantee percentage means editing a
CSV, not a rule. A test fails the build if a number in any rule or script
carries a value the driver file ships.

## What the tests actually check

The suite computes the model and asserts figures, rather than only checking that
the source parses. Each expected number is written out longhand in the test, so
a reviewer can follow the arithmetic without trusting the engine:

- Monthly base pay is headcount times the annual rate over 12.
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

[Native TM1 comparison](docs/native-comparison.md) defines 8 synthetic cells,
their independent expected values and the evidence to retain. Native comparison
is pending because no TM1 server was available; passing offline tests does not
establish agreement with a native server.

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

## Offline planning-loop benchmark

`benches/evaluate.rs` times the evaluator on fabricated workloads and the
shipped model, and records a SHA-256 fingerprint of each result, which must be
the same on every sample. Its `planning-cycle` workload constructs a small
fabricated model, changes an input, recalculates, reads the affected result
and evaluates an extended model. It checks independent arithmetic before
accepting a timing sample, so a fast incorrect result fails the run.

```bash
cargo bench --bench evaluate -- --repeats 5 --output /approved/path/benchmark.json
```

To compare two revisions, run the benchmark at each and compare the reports:
equal fingerprints show both computed the same results. The report measures
the repository's offline evaluator and includes model construction and result
checks. It is not a native TM1 comparison or an FPABench
score. The [native comparison](docs/native-comparison.md) has its own pending
server checks. The planning-loop scenario borrows the evaluation idea from
[FPABench](https://github.com/farseerdev/FPABench), without using its model or data.

## Client data

No real data belongs in this repository. The `.gitignore` blocks the file names
ledgers and payroll exports arrive under. The examples combine invented business
data with dated, sourced statutory figures. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Releases

The repository's [GitHub Releases](https://github.com/ryanduguid/planning-analytics-model/releases) page is the canonical release history. A separate changelog is intentionally not maintained.

## Author

Written by Ryan Duguid, a provisional member of Chartered Accountants ANZ,
independently, in his own time and on his own equipment. Nothing here is the
work of any employer, and no client data was used to build or test it.

Every statutory figure is traced to its source in `docs/model-assumptions.md`,
which records what was read from the legislation, what was taken from the
administering authority's own page, and who confirmed the figures that could not
be fetched.

## Licence

Licensed under the Functional Source License, Version 1.1, ALv2 Future License (FSL-1.1-ALv2). You may use, modify and redistribute it for any purpose except a Competing Use as defined in LICENSE (making it available to others in a competing commercial product or service), and each version becomes available under Apache-2.0 two years after it is made available. See [LICENSE](LICENSE) and [NOTICE](NOTICE). If you redistribute this work or build on it, keep the copyright notices in [NOTICE](NOTICE).

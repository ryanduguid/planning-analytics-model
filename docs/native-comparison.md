# Native TM1 comparison

Native comparison is pending: no TM1 / Planning Analytics server was available
for the September 2026 verification. The checks below define a small comparison
against the shipped synthetic model. Offline results must never be recorded as
native results.

`native-comparison-manifest.json` binds all ten cases to the model commit and
SHA-256 hashes of every model definition and the five synthetic input files.
It records the previously implicit input traces, comparison tolerance and
collection procedure. The offline regression tests check those hashes and the
independent arithmetic; all native evidence fields remain pending until an
authorised isolated server and raw exports are available.

## Model and inputs

Use an isolated TM1 database with this checkout's `model/` definitions and all
5 `examples/` CSV files, loaded through the model's matching processes.
Do not use a production database or client data. Record the repository commit,
TM1 server version and build, date of execution and the operator who loaded it.

Every case uses `FY2026-27`, `Budget` and `Jul`. Coordinates after those 3
elements follow the cube's declared dimension order. The constants below are
the model's synthetic planning inputs, not maintained statutory rates.

| Cube | Remaining coordinates | Independent calculation | Expected |
| --- | --- | --- | ---: |
| Workforce | CivilCo, Earthworks, Operator, Base Pay | 18 × 125,000 / 12 | 187,500.00 |
| Workforce | CivilCo, Earthworks, Operator, Superannuation Cost | 18 × 125,000 × 0.12 / 12 | 22,500.00 |
| Workforce | CivilCo, Drill and Blast, Supervisor, Superannuation Cost | 270,830 × 0.12 / 12 | 2,708.30 |
| Workforce | CivilCo, Earthworks, Operator, Payroll Tax Cost | (187,500 + 22,500) × 0.0545 | 11,445.00 |
| Revenue | CivilCo, Earthworks, Contract Revenue Amount | 5,200 × 295 | 1,534,000.00 |
| Capex | CivilCo, Excavator, Depreciation Charge | 2,400,000 / 84 | 28,571.43 |
| PnL | CivilCo, Corporate, Payroll Tax, Amount | (25,000 + 3,000) × 0.0545 − 1,200,000 × 0.0545 × 31 / 365 | -4,028.52 |
| PnL | CivilCo, All Cost Centres, Gross Margin, Amount | (1,534,000 + 180,000 + 574,000) − (260,000 + 95,000 + 162,800 + 48,100 + 48,000 + 22,000) | 1,652,100.00 |

The last case checks a consolidated node as well as leaf calculations. Read it
from a native cube view with zero suppression enabled so a missing feeder does
not escape the check. Compare currency at cents, with a maximum unrounded
difference of half a cent; retain the raw exported values.

Two payroll tax threshold cases need altered inputs, loaded into a separate
database. With the `FY2026-27,Budget,Feb,Days` row left out of `drivers.csv`,
February's CivilCo `Corporate` payroll tax should read as undefined (N/A), not a
figure, while July still reads -4,028.52, because the year's days come from
`Full Year` rather than the months; the offline evaluator exits 3 on the same
data. With CivilCo `Corporate`'s workforce removed, a zero-suppressed view of
its `FY` payroll tax should still show the -65,400 threshold credit, and the
`Group` total should include it; only the `Drivers` feeder feeds those cells
then. Open that view on a fresh feeder state before using Check Feeders, which
feeds cells itself and would hide a missing feeder.

## Loading the model into the isolated database

Two routes put `model/` into a TM1 database, and neither has been run against a
server for this repository.

1. The server's own Git integration. Put the contents of `model/` in a Git
   repository the server can reach, then run `GitInit`, `GitPull` for the
   branch and `Execute` on the plan it returns, as IBM's TM1 source
   specification describes.
2. [tm1gitpy](https://github.com/KnowledgeSeed/tm1gitpy), which works outside
   the server through its REST API. Compare an empty folder with `model/` to
   get a changeset that creates every object, then apply it to a server named
   in tm1gitpy's `tm1servers.yaml`:

   ```bash
   python -m venv .tm1git && .tm1git/bin/pip install --no-deps --require-hashes -r ci/requirements-tm1git.txt
   .tm1git/bin/tm1gitpy compare --source empty --target model --output changeset.yaml --max-workers 8
   .tm1git/bin/tm1gitpy apply --server isolated --changeset changeset.yaml
   ```

   tm1gitpy 1.2.10's `compare` fails without `--max-workers`, and with
   `--max-workers 2`, which leaves no worker for one side; 8 works. Read the
   changeset before applying it.

Either way, load the 5 CSV files through the model's processes afterwards.
The strongest evidence is a round trip: after loading, have the server push
the model back to Git (or export it with tm1gitpy) and compare that source with
`model/`, then run the cases above. The
offline half of the second route runs in CI: the `tm1git-conformance` job has
tm1gitpy read `model/` and requires it to agree with this engine on the
dimensions, elements, edges, cube dimension order and process script links.

## Offline reproduction

Install from the checkout as described in the README. For each table row, pass
the cube and complete coordinates to the existing command. For example:

```bash
pacioliscube calculate model --data examples --cell "Workforce:FY2026-27,Budget,Jul,CivilCo,Earthworks,Operator,Base Pay"
pacioliscube calculate model --data examples --cell "PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount"
```

The existing `tests/calculations.rs` works out these expectations without
copying engine output. Run the repository's normal checks:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## Complete the native record

Export these 8 cells from the isolated native model with full precision.
Keep the coordinates, native value, offline value and difference together.
Record whether every case agrees, including the suppressed-zero view check.
Investigate any mismatch before claiming compatibility. This limited comparison
would cover the named calculations, not arbitrary TM1 models or language features.

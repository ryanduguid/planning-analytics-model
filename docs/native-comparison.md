# Native TM1 comparison

Native comparison is pending: no TM1 / Planning Analytics server was available
for the September 2026 verification. The checks below define a small comparison
against the shipped synthetic model. Offline results must never be recorded as
native results.

## Model and inputs

Use an isolated TM1 database with this checkout's `model/` definitions and all
five `examples/` CSV files, loaded through the model's matching processes.
Do not use a production database or client data. Record the repository commit,
TM1 server version and build, date of execution and the operator who loaded it.

Every case uses `FY2026-27`, `Budget` and `Jul`. Coordinates after those three
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
| PnL | CivilCo, Corporate, Payroll Tax, Amount | (25,000 + 3,000) × 0.0545 − 1,200,000 × 0.0545 / 12 | -3,924.00 |
| PnL | CivilCo, All Cost Centres, Gross Margin, Amount | (1,534,000 + 180,000 + 574,000) − (260,000 + 95,000 + 162,800 + 48,100 + 48,000 + 22,000) | 1,652,100.00 |

The last case checks a consolidated node as well as leaf calculations. Read it
from a native cube view with zero suppression enabled so a missing feeder does
not escape the check. Compare currency at cents, with a maximum unrounded
difference of half a cent; retain the raw exported values.

## Offline reproduction

Install from the checkout as described in the README. For each table row, pass
the cube and complete coordinates to the existing command. For example:

```bash
pacioliscube calculate model --data examples --cell "Workforce:FY2026-27,Budget,Jul,CivilCo,Earthworks,Operator,Base Pay"
pacioliscube calculate model --data examples --cell "PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount"
```

The existing `tests/test_calculations.py` works out these expectations without
copying engine output. Run the repository's normal checks:

```bash
uv run --locked --extra dev ruff check pacioliscube tests
uv run --locked --extra dev mypy pacioliscube
uv run --locked --extra dev pytest -q
```

## Complete the native record

Export these eight cells from the isolated native model with full precision.
Keep the coordinates, native value, offline value and difference together.
Record whether every case agrees, including the suppressed-zero view check.
Investigate any mismatch before claiming compatibility. This limited comparison
would cover the named calculations, not arbitrary TM1 models or language features.

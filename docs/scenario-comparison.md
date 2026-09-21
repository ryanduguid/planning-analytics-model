# Compare two data snapshots

`pacioliscube compare` evaluates the same requested cells against two directories
using one model. It prints previous, current and difference amounts, both complete
calculation explanations and changed evidence nodes. An absent input remains
`default_zero`, so removing an explicit zero is visible even when the value is unchanged.

```powershell
uv run --locked pacioliscube compare model --previous-data examples --current-data examples --cell 'PnL:FY2026-27,Budget,FY,Group,All Cost Centres,EBITDA,Amount'
```

This reproducible baseline produces a zero difference. Copy the example CSVs to
two separate local directories and change a reviewed driver in the second to
compare scenarios. Both snapshots must use the same model dimensions and cell
coordinates. Give one `--cell` for each entity, cost centre or measure to inspect.
There is no grand total across requested cells: selecting both Group and a child
entity would otherwise double-count the child.

Differences are arithmetic evidence, not a causal allocation. Retain the input
directories alongside the JSON output. The command does not compare different
model revisions or establish agreement with a native TM1 server.

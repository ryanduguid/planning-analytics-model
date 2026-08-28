# Calculation-engine mutation baseline

This document records the bounded `mutmut` 3.7.0 pilot for
`pacioliscube/evaluate.py`. It is a measurement and test-design aid, not a CI
gate. Run it on Linux, because mutmut relies on process forking:

```bash
uv run --locked --extra dev --python 3.12 mutmut run
uv run --locked --extra dev --python 3.12 mutmut results
```

## Baseline

GitHub Actions run [33086803423](https://github.com/ryanduguid/PaciolisCube/actions/runs/33086803423)
tested commit `73c411f8ff5af6f4ca30f2e17e0ae39f49746c68` on Ubuntu with Python 3.12.
The complete run took 38 seconds and produced:

- 424 generated mutants
- 312 killed
- 68 survived
- 44 with no selected test
- 0 timeouts, suspicious results, skips or errors

The detection rate was 82.11% among the 380 mutants that had a selected test,
or 73.58% when mutants without a selected test remain in the denominator.
Mutant `pacioliscube.evaluate.xǁ_Engineǁevaluate_expression__mutmut_34`
changed multiplication into division and was killed. This independently
confirms that the seeded multiplication property detects a weakened calculation
rule.

## Survivor ledger

Every one of the 68 true survivors from follow-up run
[33087043111](https://github.com/ryanduguid/PaciolisCube/actions/runs/33087043111)
is covered by one grouped disposition below. Ranges are inclusive.

| Mutant identifiers | Count | Disposition |
| --- | ---: | --- |
| `CellStore.set` 2-5 | 4 | Cosmetic exception-message mutations. Keep behavioural assertions and exclude message-only mutants if a future gate is introduced. |
| `CellStore.set` 12 | 1 | Add a test proving `items()` preserves the first coordinate spelling and case. |
| `_dimension_of` 8 | 1 | Add an exactly-two-dimensions ambiguity case. |
| `_dimension_of` 9-12 | 4 | Diagnostic-message construction. Add one semantic assertion that both conflicting dimensions are named; treat remaining wording changes as cosmetic. |
| `_area_positions` 14 | 1 | Add conflicting elements from the same dimension and assert rejection. |
| `_area_positions` 15 | 1 | Diagnostic-message removal, covered by the same semantic rejection test. |
| `_Engine.matching_rule` 18-20 | 3 | Exercise consolidated `C` rules and an inapplicable rule before an applicable rule. |
| `_Engine.value` 4 and 21 | 2 | Diagnostic-only changes for missing cubes and circular-reference chains. Assert the error category and useful names, not exact prose. |
| `_Engine.consolidated` 9 and 24-27 | 5 | Exercise a hierarchy whose first dimension is a leaf and second is consolidated, plus the public leaf-coordinate fallback path. |
| `_Engine.evaluate_expression` 38 | 1 | Add a non-zero safe-division example; the survivor changes safe division into multiplication. |
| `_Engine.evaluate_expression` 44-46 and 48 | 4 | Division and unsupported-operator diagnostics. Add semantic error assertions and ignore punctuation-only changes. |
| `_Engine.evaluate_expression` 49-66 | 18 | Add public-model properties for both `IF` branches with distinct, non-zero values. |
| `_Engine.evaluate_expression` 67-74 | 8 | Add public-model properties for true and false comparison results. |
| `_Engine.evaluate_expression` 75-76 | 2 | Unsupported-node diagnostics. Assert the node category without freezing exact wording. |
| `_Engine.resolve_reference` 18, 20 and 29 | 3 | Reference diagnostics. Assert the failing reference and operator rather than the full message. |
| `_rule_targets` 10 and 20-22 | 4 | Add cardinality properties around the materialisation cap and multiple selectors. |
| `_rule_targets` 23 | 1 | Diagnostic-message removal, covered by the materialisation-cap assertion. |
| `evaluate` 8, 11-13 and 17 | 5 | Add multi-cube evaluation with a ruleless cube first, consolidated `C` rules and selector-driven target materialisation. |

The 44 mutants without selected tests are a separate selection-coverage gap.
Before enabling a mutation threshold, inspect that set and either connect the
relevant tests to the mutated functions or explicitly exclude generated,
diagnostic-only or unreachable mutations.

## Decision

Keep the pinned mutation dependency and the narrow `only_mutate` configuration.
Do not add the pilot to CI yet. Re-run it manually on Linux after addressing the
survivor clusters, then consider a threshold only when the no-test set and
diagnostic-only policy are explicit.

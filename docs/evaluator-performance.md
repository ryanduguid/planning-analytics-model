# Evaluator performance

This page records the Python engine's rule index, measured before the Rust
command line replaced it in version 1.0.0. The rule index, its first-match
order and its lazy area resolution carried over unchanged. Time the Rust
engine with `cargo bench --bench evaluate`, as the README describes.

The rule index reduces repeated rule scanning in the Python Decimal evaluator.
On 5 October 2026, the 65,000-cell fabricated workload ran in 1.08 seconds,
compared with 2.47 seconds at revision
`b4da44a8665128113366ab488683db6f5a2173b9`.

Rules are grouped by the dimension positions their areas select. Each group
maps an element tuple to its earliest source rule. Lookup chooses the lowest
matching source index and stops once subsequent groups cannot beat it. Groups
are inserted in source order, so their first rule supplies that lower bound.
Leaf and consolidated requests
have separate indexes. Each index extends only after its prepared prefix
misses, so a matching earlier rule still hides an invalid later area.
Python casefolds selector names once and reuses the coordinate key already
prepared for memoisation. Indexes last for one engine invocation.

## Measurements

These are medians of 7 interleaved repetitions on Linux x86-64, CPython 3.14.7,
under a bounded laptop job. Every timed public call creates a fresh engine,
including index preparation. Model loading and output fingerprinting are
outside the timed interval. Result teardown finishes before the next sample
starts. Input fixtures are fabricated. Batch calls request
the last 1,000 returned cells; explanations request the last 64.

| Workload | Before, seconds | Python index, seconds | Speedup |
|---|---:|---:|---:|
| Shipped model, 16,418 returned cells | 0.6703 | 0.6306 | 1.06× |
| Shipped model batch | 0.2327 | 0.2201 | 1.06× |
| Shipped model explanation | 0.0203 | 0.0199 | 1.02× |
| 100 entities, 4 rules | 0.0081 | 0.0071 | 1.14× |
| 500 entities, 16 rules | 0.1826 | 0.1329 | 1.37× |
| 1,000 entities, 64 rules | 2.4667 | 1.0811 | 2.28× |
| Large batch | 0.1010 | 0.0196 | 5.15× |
| Large explanation | 0.0094 | 0.0042 | 2.25× |
| 64 selector shapes, early matches after a miss | 0.0097 | 0.0101 | 0.96× |

Speedup is the baseline median divided by the indexed median. The final case
prepares 64 different sets of selected dimensions with an initial miss, then
requests 1,000 cells matching the first rule. Its indexed median was 4.5%
slower, an additional 0.43 milliseconds. The index therefore benefits repeated
rule searches most; it can add overhead when an early rule already matches.

[Raw samples and fingerprints](../benchmarks/results/2026-10-05.json) include
minimum and maximum timings, evaluator source hashes and a separate Python
allocation measurement. The large evaluation's peak allocation increased from
52,167,356 to 52,177,058 bytes. The selector-shape case increased from 602,601
to 641,866 bytes. These results describe the supplied workloads on one machine.

A separate Rust/PyO3 experiment tested only the string-based matcher. It is
excluded from this implementation and its performance evidence. The delivered
Python implementation used the index and had no runtime dependencies. These results
do not assess a complete Rust port or an integer-ID native matcher.

## Reproduce the Python comparison

The comparison ran `benchmarks/evaluate.py` from the last Python release,
0.3.0, against the evaluator at revision
`b4da44a8665128113366ab488683db6f5a2173b9`. Both files remain in the
repository history at tag `v0.3.0`; the baseline file is imported as Python
code and must come from a trusted revision. The benchmark compares ordered stores, batch values and complete
explanation trees, encoding every Decimal through `as_tuple()` and tagging
container types. It verifies the public return types and stops on a mismatch,
including exponent, signed-zero, tuple/list or evidence-order differences.
The `--memory` option runs allocation measurements after the timing samples.

Focused tests cover first-match precedence, overlapping areas, qualifier
order, lazy invalid areas, Unicode casefolding, fresh rules and Decimal
contexts and traps. This is parity with the offline evaluator. No native
TM1 server comparison was performed.

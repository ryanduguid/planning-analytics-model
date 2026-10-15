"""Benchmark fingerprints and sample lifetimes must support the reported claims."""

from benchmarks import evaluate as benchmark
from pacioliscube import evaluate as evaluator
from pacioliscube.evaluate import CellStore


def test_fingerprint_preserves_container_types():
    assert benchmark.fingerprint((1, 2)) != benchmark.fingerprint([1, 2])
    assert benchmark.fingerprint(CellStore()) != benchmark.fingerprint([])
    assert benchmark.fingerprint({"coordinate": ("Red",)}) != benchmark.fingerprint({
        "coordinate": ["Red"],
    })


def test_planning_cycle_checks_changed_inputs_and_model_structure():
    from decimal import Decimal

    assert benchmark.planning_cycle(evaluator) == (
        Decimal("0.1") * 2, Decimal("2") * 2, Decimal("0.1") * 2 + 2,
    )


def test_planning_cycle_refuses_fast_incorrect_results(monkeypatch):
    import pytest

    monkeypatch.setattr(evaluator, "evaluate", lambda model, inputs: inputs)
    with pytest.raises(RuntimeError, match="independent expectation"):
        benchmark.planning_cycle(evaluator)


def test_preceding_result_teardown_is_outside_the_next_sample(monkeypatch):
    clock = [0]

    class Result(dict):
        def __del__(self):
            clock[0] += 100

    def operation(_module):
        clock[0] += 1
        return Result(value=1)

    monkeypatch.setattr(benchmark.time, "perf_counter", lambda: clock[0])
    report = benchmark.compare("lifetime", {"fake": object()}, operation, 2, False)
    # Each operation takes 1 simulated second. Its 100-second teardown must
    # happen after the clock reading and before the following sample begins.
    assert report["backends"]["fake"]["seconds"] == [1, 1]
    assert clock[0] == 202

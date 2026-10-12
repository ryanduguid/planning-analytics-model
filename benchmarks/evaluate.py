"""Compare fresh public calls against the frozen evaluator on fabricated data."""

import argparse
import gc
import hashlib
import importlib.util
import itertools
import json
import platform
import statistics
import sys
import time
import tracemalloc
from decimal import Decimal
from pathlib import Path

from pacioliscube import evaluate as current
from pacioliscube.data import load_data
from pacioliscube.evaluate import CellStore
from pacioliscube.model import Cube, Dimension, Element, Hierarchy, Model, load_model
from pacioliscube.rules import parse_rules


def workload(entities, rules):
    root = Path.cwd()
    entity_names = [f"Entity{i}" for i in range(entities)]
    measures = ["Input"] + [f"Output{i}" for i in range(rules)]
    dimensions = {}
    for name, elements in (("Entity", entity_names), ("Measure", measures)):
        source = root / f"synthetic-{name}.json"
        hierarchy = Hierarchy(name, name, [Element(x, "Numeric") for x in elements], [], source)
        dimensions[name] = Dimension(name, {name: hierarchy}, source)
    text = "\n".join(
        f"['Output{i}'] = N: IF(['Input'] > 0, ['Input'] * 2 + {i}, ['Input'] / 0);"
        for i in range(rules)
    )
    rules_path = root / "synthetic.rules"
    cube = Cube("Synthetic", ("Entity", "Measure"), parse_rules(text, rules_path), rules_path, rules_path)
    model = Model("Synthetic", root, dimensions, {cube.name: cube}, {}, [])
    store = CellStore()
    for index, entity in enumerate(entity_names):
        store.set(cube.name, (entity, "Input"), Decimal(index + 1) / Decimal(10))
    return model, store

def load_evaluator(path):
    spec = importlib.util.spec_from_file_location(Path(path).stem, path)
    if spec is None or spec.loader is None:
        raise ValueError("baseline must be a trusted Python source file")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def selector_shapes():
    root = Path.cwd()
    dimensions = {}
    for name, elements in [("Entity", [f"E{i}" for i in range(1000)])] + [
        (f"D{i}", [f"D{i}a", f"D{i}b"]) for i in range(7)
    ]:
        source = root / f"{name}.json"
        hierarchy = Hierarchy(name, name, [Element(x, "Numeric") for x in elements], [], source)
        dimensions[name] = Dimension(name, {name: hierarchy}, source)
    areas = [("D0a",)] + [
        tuple(f"D{i}a" for i in subset)
        for length in range(1, 7) for subset in itertools.combinations(range(1, 7), length)
    ]
    text = "\n".join(f"{list(area)!r} = N: {i + 1};" for i, area in enumerate(areas))
    rules_path = root / "shape.rules"
    cube = Cube("Shapes", tuple(dimensions), parse_rules(text, rules_path), rules_path, rules_path)
    model = Model("Shapes", root, dimensions, {cube.name: cube}, {}, [])
    miss = ("Shapes", ("E0", *(f"D{i}b" for i in range(7))))
    early = [("Shapes", (f"E{i}", "D0a", *(f"D{j}b" for j in range(1, 7))))
             for i in range(1000)]
    return model, CellStore(), [miss, *early]


def exact(value):
    if type(value).__name__ == "CellStore":
        return ("CellStore", [exact(item) for item in value.items()])
    if isinstance(value, Decimal):
        return ("Decimal", tuple(value.as_tuple()))
    if isinstance(value, dict):
        return (type(value).__name__, [(exact(key), exact(item)) for key, item in value.items()])
    if isinstance(value, tuple):
        return ("tuple", type(value).__name__, [exact(item) for item in value])
    if isinstance(value, list):
        return ("list", type(value).__name__, [exact(item) for item in value])
    return (type(value).__name__, value)


def fingerprint(value):
    return hashlib.sha256(repr(exact(value)).encode("utf-8")).hexdigest()


def copy_inputs(backend, store):
    result = backend.CellStore()
    for cube, coordinate, value in store.items():
        result.set(cube, coordinate, value)
    return result


def compare(name, modules, operation, repeats, memory, return_type=None):
    samples = {key: [] for key in modules}
    hashes = {}
    names = list(modules)
    for repeat in range(repeats):
        for key in names[repeat % len(names):] + names[:repeat % len(names)]:
            gc.collect()
            started = time.perf_counter()
            result = operation(modules[key])
            samples[key].append(time.perf_counter() - started)
            expected = modules[key].CellStore if return_type == "store" else return_type
            if expected is not None and type(result) is not expected:
                raise TypeError(f"unexpected return type for {name}: {key}")
            signature = fingerprint(result)
            if hashes and signature != next(iter(hashes.values())):
                raise RuntimeError(f"output differs for {name}: {key}")
            hashes[key] = signature
            del result
    report = {"sha256": next(iter(hashes.values())), "backends": {}}
    for key, module in modules.items():
        peak = None
        if memory:
            gc.collect()
            tracemalloc.start()
            try:
                operation(module)
                _, peak = tracemalloc.get_traced_memory()
            finally:
                tracemalloc.stop()
        report["backends"][key] = {
            "seconds": samples[key], "median_seconds": statistics.median(samples[key]),
            "min_seconds": min(samples[key]), "max_seconds": max(samples[key]),
            "python_peak_bytes": peak,
        }
    print(name, {key: row["median_seconds"] for key, row in report["backends"].items()}, flush=True)
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True, help="trusted baseline evaluate.py")
    parser.add_argument("--baseline-revision")
    parser.add_argument("--memory", action="store_true")
    parser.add_argument("--output", required=True)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    modules = {"baseline": load_evaluator(args.baseline)}
    modules["indexed"] = current
    report = {"python": platform.python_version(), "platform": platform.platform(),
              "baseline_revision": args.baseline_revision,
              "source_sha256": {
                  name: hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest()
                  for name, module in modules.items()
              }, "workloads": {}}
    model = load_model(Path("model"))
    store = load_data(model, Path("examples"))
    cases = [("repository-model", model, store)]
    for name, entities, rules in (("small", 100, 4), ("medium", 500, 16), ("large", 1000, 64)):
        cases.append((name, *workload(entities, rules)))
    for name, model, store in cases:
        inputs = {backend: copy_inputs(backend, store) for backend in modules.values()}
        report["workloads"][name] = compare(
            name, modules, lambda backend: backend.evaluate(model, inputs[backend]),
            args.repeats, args.memory, "store",
        )
        if name in ("repository-model", "large"):
            cube_names = {cube.name.casefold(): cube.name for cube in model.cubes.values()}
            cells = [(cube_names[cube], coordinate)
                     for cube, coordinate, _ in modules["baseline"].evaluate(model, store).items()]
            for operation, selected in (("batch", cells[-1000:]), ("explain", cells[-64:])):
                def call(backend, operation=operation, selected=selected):
                    if operation == "batch":
                        return list(backend.consolidate_many(model, inputs[backend], selected))
                    return backend.explain(model, inputs[backend], selected)
                report["workloads"][f"{name}-{operation}"] = compare(
                    f"{name}-{operation}", modules, call, args.repeats, args.memory,
                    list if operation == "batch" else dict,
                )
    model, store, cells = selector_shapes()
    inputs = {backend: copy_inputs(backend, store) for backend in modules.values()}
    report["workloads"]["selector-shapes-early-match"] = compare(
        "selector-shapes-early-match", modules,
        lambda backend: list(backend.consolidate_many(model, inputs[backend], cells)),
        args.repeats, args.memory, list,
    )
    Path(args.output).write_text(json.dumps(report, indent=2), encoding="utf-8")  # NOSONAR: developer benchmark CLI; the person running it chooses where its report goes


if __name__ == "__main__":
    main()

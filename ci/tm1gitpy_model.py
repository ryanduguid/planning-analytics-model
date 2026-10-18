"""Write tm1gitpy's reading of a model tree as JSON, for the conformance test.

usage: python ci/tm1gitpy_model.py MODEL_DIR OUTPUT.json

Run from a scratch directory: tm1gitpy keeps a SQLite cache under .tm1gitpy/
in the working directory. tests/model.rs compares the output with the
engine's own reading of the same tree. Weights are written as strings so no
float conversion stands between the two readers.
"""

import json
import sys

from tm1_git_py.services import deserializer


def main() -> int:
    model_dir, output = sys.argv[1], sys.argv[2]
    model, errors = deserializer.deserialize_model(model_dir, max_workers=2)
    tree = model.to_dict()
    for dimension in tree["dimensions"]:
        for hierarchy in dimension["hierarchies"]:
            for edge in hierarchy["edges"]:
                if "Weight" in edge:
                    edge["Weight"] = str(edge["Weight"])
    document = {
        "errors": {str(key): str(value) for key, value in errors.items()},
        "dimensions": tree["dimensions"],
        "cubes": tree["cubes"],
        "processes": [
            {"name": process["name"], "code_link": process["code_link"]} for process in tree["processes"]
        ],
    }
    with open(output, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, default=str)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

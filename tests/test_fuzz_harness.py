"""The fuzz target's seed corpus replays on every platform, and its checks bite."""

from pathlib import Path
from unittest import mock

import pytest

from fuzz import rules_parser

CORPUS = Path(__file__).resolve().parents[1] / "fuzz" / "corpus"


@pytest.mark.parametrize("seed", sorted(CORPUS.iterdir()), ids=lambda path: path.name)
def test_seed_corpus(seed):
    rules_parser.test_one_input(seed.read_bytes())


def test_unexpected_parser_errors_escape():
    with mock.patch.object(rules_parser, "parse_rules", side_effect=RuntimeError("bug")):
        with pytest.raises(RuntimeError, match="bug"):
            rules_parser.test_one_input(b"['Amount'] = 1;")


def test_a_parser_that_changes_its_answer_is_a_failure():
    answers = iter(["first", "second"])
    with mock.patch.object(rules_parser, "parse_rules", side_effect=lambda text, path: next(answers)):
        with pytest.raises(AssertionError, match="different results"):
            rules_parser.test_one_input(b"['Amount'] = 1;")

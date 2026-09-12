"""Parsing TM1 rule text into an expression tree."""

from decimal import Decimal
from pathlib import Path

import pytest

from pacioliscube.rules import (
    BinaryOp,
    CellRef,
    Comparison,
    IfExpr,
    Number,
    RuleSyntaxError,
    parse_rules,
)

MINI = Path(__file__).parent / "fixtures" / "mini"
INLINE = Path("inline.rules")


def load():
    path = MINI / "cubes" / "Sales.rules"
    return parse_rules(path.read_text(encoding="utf-8"), path)


def test_skipcheck_is_detected():
    assert load().skipcheck is True


def test_doubled_quotes_decode_as_one_apostrophe_in_names():
    rule = parse_rules("['O''Brien'] = N: DB('Owner''s cube', 'O''Brien');", INLINE).rules[0]
    assert rule.area.selectors == (("O'Brien",),)
    assert rule.expression == CellRef("Owner's cube", ("O'Brien",))


@pytest.mark.parametrize("text", ["['O''Brien] = N: 1;", "['O''\nBrien'] = N: 1;"])
def test_escaped_quotes_still_require_a_closing_quote_on_the_same_line(text):
    with pytest.raises(RuleSyntaxError, match="unterminated quoted name"):
        parse_rules(text, INLINE)


def test_a_file_without_skipcheck_says_so():
    assert parse_rules("['A'] = N: 1;", INLINE).skipcheck is False


def test_rules_and_feeders_are_separated():
    ruleset = load()
    assert len(ruleset.rules) == 2
    assert len(ruleset.feeders) == 2


def test_area_and_qualifier_parse():
    rule = load().rules[0]
    assert rule.area.qualifier == "N"
    assert rule.area.selectors == (("Amount",),)


def test_expression_tree_shape():
    rule = load().rules[0]
    assert isinstance(rule.expression, BinaryOp)
    assert rule.expression.op == "*"
    assert isinstance(rule.expression.left, CellRef)
    assert rule.expression.left.coordinates == ("Units",)


def test_db_reference_records_its_cube_and_current_element_marker():
    rule = load().rules[1]
    reference = rule.expression.right
    assert isinstance(reference, CellRef)
    assert reference.cube == "Cost"
    assert reference.coordinates == ("!Colour", "Amount")


def test_feeders_record_source_and_target_areas():
    feeder = load().feeders[0]
    assert feeder.area.selectors == (("Units",),)
    assert feeder.target.selectors == (("Amount",),)
    assert feeder.target_cube is None


def test_a_cross_cube_feeder_records_its_target_cube():
    text = "FEEDERS;\n['Amount'] => DB('PnL', !Colour, 'Wages');"
    feeder = parse_rules(text, INLINE).feeders[0]
    assert feeder.target_cube == "PnL"
    assert feeder.target.selectors == (("!Colour",), ("Wages",))


def test_numbers_parse_as_decimal_keeping_their_written_precision():
    ruleset = parse_rules("['A'] = N: 1.10;", INLINE)
    assert ruleset.rules[0].expression == Number(Decimal("1.10"))


def test_multiplication_binds_tighter_than_addition():
    rule = parse_rules("['A'] = N: 1 + 2 * 3;", INLINE).rules[0]
    assert rule.expression.op == "+"
    assert rule.expression.right.op == "*"


def test_parentheses_override_precedence():
    rule = parse_rules("['A'] = N: (1 + 2) * 3;", INLINE).rules[0]
    assert rule.expression.op == "*"
    assert rule.expression.left.op == "+"


def test_subtraction_is_left_associative():
    rule = parse_rules("['A'] = N: 10 - 3 - 2;", INLINE).rules[0]
    assert rule.expression.op == "-"
    assert rule.expression.left.op == "-"
    assert rule.expression.right == Number(Decimal("2"))


def test_unary_minus_parses():
    rule = parse_rules("['A'] = N: -5 + 1;", INLINE).rules[0]
    assert rule.expression.left == Number(Decimal("-5"))


def test_if_expression_parses_with_a_comparison():
    rule = parse_rules("['A'] = N: IF(['B'] > 0, ['B'], 0);", INLINE).rules[0]
    assert isinstance(rule.expression, IfExpr)
    assert isinstance(rule.expression.condition, Comparison)
    assert rule.expression.condition.op == ">"


def test_a_multi_dimension_area_keeps_one_selector_group_per_position():
    rule = parse_rules("['Budget', 'Amount'] = N: 1;", INLINE).rules[0]
    assert rule.area.selectors == (("Budget",), ("Amount",))


def test_a_consolidated_qualifier_is_recorded():
    rule = parse_rules("['Rate'] = C: 1;", INLINE).rules[0]
    assert rule.area.qualifier == "C"


def test_an_unqualified_rule_records_an_empty_qualifier():
    rule = parse_rules("['Rate'] = 1;", INLINE).rules[0]
    assert rule.area.qualifier == ""


def test_comments_are_ignored():
    ruleset = parse_rules("# a comment\n['A'] = N: 1;  # trailing\n", INLINE)
    assert len(ruleset.rules) == 1


def test_unsupported_function_is_a_syntax_error_naming_the_line():
    text = "['A'] = N: 1;\n['B'] = N: ATTRN('Colour', !Colour, 'Rate');"
    with pytest.raises(RuleSyntaxError) as caught:
        parse_rules(text, INLINE)
    message = str(caught.value)
    assert "ATTRN" in message
    assert "inline.rules" in message
    assert "line 2" in message


def test_missing_semicolon_is_a_syntax_error():
    with pytest.raises(RuleSyntaxError):
        parse_rules("['A'] = N: 1 + 2", INLINE)


def test_an_unterminated_string_is_a_syntax_error():
    with pytest.raises(RuleSyntaxError):
        parse_rules("['A] = N: 1;", INLINE)


def test_a_statement_without_an_area_target_is_a_syntax_error():
    with pytest.raises(RuleSyntaxError):
        parse_rules("Amount = N: 1;", INLINE)

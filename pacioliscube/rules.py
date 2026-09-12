"""Tokenise and parse TM1 rule text into an expression tree.

The grammar accepted here is a deliberately narrow subset of TM1 rules, wide
enough for a driver based planning model and no wider. Anything outside it is a
load error naming the file and the line, never a silently wrong number.

    statement    := area '=' qualifier ':' expression ';'
                  | area '=' expression ';'
    feeder       := area '=>' area ';'
                  | area '=>' cellref ';'
    area         := '[' selector { ',' selector } ']'
    selector     := "'" name "'"
    qualifier    := 'N' | 'C'
    expression   := term { ('+'|'-') term }
    term         := factor { ('*'|'/'|'\\') factor }
    factor       := number | cellref | '(' expression ')' | ifexpr | '-' factor
    cellref      := 'DB' '(' "'" cube "'" { ',' argument } ')' | area
    ifexpr       := 'IF' '(' comparison ',' expression ',' expression ')'
    comparison   := expression ('='|'<'|'>'|'<='|'>='|'<>') expression
    argument     := "'" literal "'" | '!' dimension

``!Dimension`` inside a ``DB()`` argument means the current element of that
dimension, which is how a TM1 rule addresses the cell being calculated.

TM1 keeps two division operators and so does this parser. The backslash is the
safe divide, giving zero for a zero divisor, and the forward slash is the plain
divide, which the evaluator refuses to perform when the divisor is zero rather
than inventing a number.
"""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import NamedTuple, Optional, Union

KEYWORDS = {"DB", "IF", "SKIPCHECK", "FEEDERS", "UNDEFVALS", "FEEDSTRINGS"}
COMPARISONS = {"=", "<", ">", "<=", ">=", "<>"}


class RuleSyntaxError(ValueError):
    """Raised when rule text steps outside the supported grammar."""


class Number(NamedTuple):
    value: Decimal


class CellRef(NamedTuple):
    cube: Optional[str]
    coordinates: tuple[str, ...]


class BinaryOp(NamedTuple):
    op: str
    left: "Expr"
    right: "Expr"


class Comparison(NamedTuple):
    op: str
    left: "Expr"
    right: "Expr"


class IfExpr(NamedTuple):
    condition: Comparison
    then_expr: "Expr"
    else_expr: "Expr"


Expr = Union[Number, CellRef, BinaryOp, Comparison, IfExpr]


class Area(NamedTuple):
    qualifier: str
    selectors: tuple[tuple[str, ...], ...]


class Rule(NamedTuple):
    area: Area
    expression: Expr
    source_line: int


class Feeder(NamedTuple):
    area: Area
    target: Area
    source_line: int
    # A feeder may point into another cube, written as an => DB('Cube', ...)
    # target, which is how TM1 feeds a reporting cube from its source cubes.
    target_cube: Optional[str] = None


class RuleSet(NamedTuple):
    skipcheck: bool
    rules: tuple[Rule, ...]
    feeders: tuple[Feeder, ...]


class Token(NamedTuple):
    kind: str
    text: str
    line: int


def _tokenise(text: str, path: Path) -> list[Token]:
    tokens: list[Token] = []
    line = 1
    index = 0
    length = len(text)
    while index < length:
        character = text[index]
        if character == "\n":
            line += 1
            index += 1
            continue
        if character in " \t\r":
            index += 1
            continue
        if character == "#":
            while index < length and text[index] != "\n":
                index += 1
            continue
        if character == "'":
            cursor = index + 1
            parts: list[str] = []
            while True:
                end = text.find("'", cursor)
                if end == -1 or "\n" in text[cursor:end]:
                    raise RuleSyntaxError(f"{path} line {line}: unterminated quoted name")
                parts.append(text[cursor:end])
                if not text.startswith("''", end):
                    break
                parts.append("'")
                cursor = end + 2
            tokens.append(Token("string", "".join(parts), line))
            index = end + 1
            continue
        if character.isdigit() or (
            character == "." and index + 1 < length and text[index + 1].isdigit()
        ):
            start = index
            while index < length and (text[index].isdigit() or text[index] == "."):
                index += 1
            tokens.append(Token("number", text[start:index], line))
            continue
        if character.isalpha() or character == "_":
            start = index
            while index < length and (text[index].isalnum() or text[index] == "_"):
                index += 1
            tokens.append(Token("name", text[start:index], line))
            continue
        if text.startswith("=>", index):
            tokens.append(Token("op", "=>", line))
            index += 2
            continue
        if text.startswith("<>", index) or text.startswith("<=", index) or text.startswith(">=", index):
            tokens.append(Token("op", text[index:index + 2], line))
            index += 2
            continue
        if character in "[](),;=+-*/\\<>:!":
            tokens.append(Token("op", character, line))
            index += 1
            continue
        raise RuleSyntaxError(f"{path} line {line}: unexpected character {character!r}")
    tokens.append(Token("end", "", line))
    return tokens


class _Parser:
    def __init__(self, tokens: list[Token], path: Path) -> None:
        self.tokens = tokens
        self.path = path
        self.position = 0

    @property
    def current(self) -> Token:
        return self.tokens[self.position]

    def fail(self, message: str, token: Optional[Token] = None) -> RuleSyntaxError:
        token = token or self.current
        return RuleSyntaxError(f"{self.path} line {token.line}: {message}")

    def advance(self) -> Token:
        token = self.tokens[self.position]
        self.position += 1
        return token

    def accept(self, kind: str, text: Optional[str] = None) -> Optional[Token]:
        token = self.current
        if token.kind != kind:
            return None
        if text is not None and token.text.upper() != text.upper():
            return None
        return self.advance()

    def expect(self, kind: str, text: Optional[str] = None) -> Token:
        token = self.accept(kind, text)
        if token is None:
            wanted = text or kind
            raise self.fail(f"expected {wanted!r} but found {self.current.text or 'end of file'!r}")
        return token

    def parse_area(self) -> Area:
        self.expect("op", "[")
        selectors: list[tuple[str, ...]] = []
        if self.accept("op", "]") is None:
            while True:
                name = self.expect("string")
                selectors.append((name.text,))
                if self.accept("op", ",") is None:
                    break
            self.expect("op", "]")
        return Area("", tuple(selectors))

    def parse_primary(self) -> Expr:
        token = self.current
        if token.kind == "number":
            self.advance()
            return Number(_decimal(token.text, self.path, token.line))
        if token.kind == "op" and token.text == "-":
            self.advance()
            operand = self.parse_primary()
            if isinstance(operand, Number):
                return Number(-operand.value)
            return BinaryOp("-", Number(Decimal("0")), operand)
        if token.kind == "op" and token.text == "(":
            self.advance()
            expression = self.parse_expression()
            self.expect("op", ")")
            return expression
        if token.kind == "op" and token.text == "[":
            area = self.parse_area()
            return CellRef(None, tuple(selector[0] for selector in area.selectors))
        if token.kind == "name":
            upper = token.text.upper()
            if upper == "DB":
                return self.parse_db()
            if upper == "IF":
                return self.parse_if()
            raise self.fail(f"{token.text} is outside the supported rule grammar")
        raise self.fail(f"unexpected {token.text or 'end of file'!r} in an expression")

    def parse_db(self) -> CellRef:
        self.expect("name", "DB")
        self.expect("op", "(")
        cube = self.expect("string").text
        coordinates: list[str] = []
        while self.accept("op", ",") is not None:
            if self.accept("op", "!") is not None:
                dimension = self.expect("name")
                coordinates.append("!" + dimension.text)
            else:
                coordinates.append(self.expect("string").text)
        self.expect("op", ")")
        return CellRef(cube, tuple(coordinates))

    def parse_if(self) -> IfExpr:
        self.expect("name", "IF")
        self.expect("op", "(")
        condition = self.parse_comparison()
        self.expect("op", ",")
        then_expr = self.parse_expression()
        self.expect("op", ",")
        else_expr = self.parse_expression()
        self.expect("op", ")")
        return IfExpr(condition, then_expr, else_expr)

    def parse_comparison(self) -> Comparison:
        left = self.parse_expression()
        token = self.current
        if token.kind != "op" or token.text not in COMPARISONS:
            raise self.fail("expected a comparison operator inside IF")
        self.advance()
        right = self.parse_expression()
        return Comparison(token.text, left, right)

    def parse_term(self) -> Expr:
        expression = self.parse_primary()
        while self.current.kind == "op" and self.current.text in ("*", "/", "\\"):
            operator = self.advance().text
            expression = BinaryOp(operator, expression, self.parse_primary())
        return expression

    def parse_expression(self) -> Expr:
        expression = self.parse_term()
        while self.current.kind == "op" and self.current.text in ("+", "-"):
            operator = self.advance().text
            expression = BinaryOp(operator, expression, self.parse_term())
        return expression


def _decimal(text: str, path: Path, line: int) -> Decimal:
    try:
        return Decimal(text)
    except InvalidOperation as error:
        raise RuleSyntaxError(f"{path} line {line}: {text!r} is not a number") from error


def parse_rules(text: str, path: Path) -> RuleSet:
    """Parse a rules file into its SKIPCHECK flag, its rules and its feeders."""
    tokens = _tokenise(text, path)
    parser = _Parser(tokens, path)
    skipcheck = False
    rules: list[Rule] = []
    feeders: list[Feeder] = []
    in_feeders = False

    while parser.current.kind != "end":
        token = parser.current
        if token.kind == "name":
            upper = token.text.upper()
            if upper in ("SKIPCHECK", "FEEDERS", "UNDEFVALS", "FEEDSTRINGS"):
                parser.advance()
                parser.expect("op", ";")
                if upper == "SKIPCHECK":
                    skipcheck = True
                elif upper == "FEEDERS":
                    in_feeders = True
                continue
            raise parser.fail(f"{token.text} is outside the supported rule grammar", token)
        if token.kind != "op" or token.text != "[":
            raise parser.fail(
                f"a statement must start with an area in square brackets, found {token.text or 'end of file'!r}",
                token,
            )
        line = token.line
        area = parser.parse_area()
        if parser.accept("op", "=>") is not None:
            if parser.current.kind == "name" and parser.current.text.upper() == "DB":
                reference = parser.parse_db()
                target = Area("", tuple((element,) for element in reference.coordinates))
                parser.expect("op", ";")
                feeders.append(Feeder(area, target, line, reference.cube))
            else:
                target = parser.parse_area()
                parser.expect("op", ";")
                feeders.append(Feeder(area, target, line))
            continue
        if in_feeders:
            raise parser.fail("a statement after FEEDERS must be a feeder using =>", token)
        parser.expect("op", "=")
        qualifier = ""
        if parser.current.kind == "name" and parser.current.text.upper() in ("N", "C"):
            lookahead = parser.tokens[parser.position + 1]
            if lookahead.kind == "op" and lookahead.text == ":":
                qualifier = parser.advance().text.upper()
                parser.advance()
        expression = parser.parse_expression()
        parser.expect("op", ";")
        rules.append(Rule(Area(qualifier, area.selectors), expression, line))

    return RuleSet(skipcheck, tuple(rules), tuple(feeders))

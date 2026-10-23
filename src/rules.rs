//! Tokenise and parse TM1 rule text into an expression tree.
//!
//! The grammar accepted here is a deliberately narrow subset of TM1 rules, wide
//! enough for a driver based planning model and no wider. Anything outside it is a
//! load error naming the file and the line, never a silently wrong number.
//!
//! ```text
//! statement    := area '=' qualifier ':' expression ';'
//!               | area '=' expression ';'
//! feeder       := area '=>' area ';'
//!               | area '=>' cellref ';'
//! area         := '[' selector { ',' selector } ']'
//! selector     := "'" name "'"
//! qualifier    := 'N' | 'C'
//! expression   := term { ('+'|'-') term }
//! term         := factor { ('*'|'/'|'\') factor }
//! factor       := number | cellref | '(' expression ')' | ifexpr | '-' factor
//! cellref      := 'DB' '(' "'" cube "'" { ',' argument } ')' | area
//! ifexpr       := 'IF' '(' comparison ',' expression ',' expression ')'
//! comparison   := expression ('='|'<'|'>'|'<='|'>='|'<>') expression
//! argument     := "'" literal "'" | '!' dimension
//! ```
//!
//! `!Dimension` inside a `DB()` argument means the current element of that
//! dimension, which is how a TM1 rule addresses the cell being calculated.
//!
//! TM1 keeps two division operators and so does this parser. The backslash is
//! the safe divide, giving zero for a zero divisor, and the forward slash is the
//! plain divide, which the evaluator refuses to perform when the divisor is zero
//! rather than inventing a number.
//!
//! The Python parser recursed once per call and stopped at the interpreter's
//! recursion limit. The parser here counts the same calls, so a rules file is
//! refused as nested too deeply at exactly the depth Python refused it.

use crate::decimal::Decimal;
use crate::error::{Error, Uncaught};
use crate::py;

/// A rules file outside the supported grammar: Python's `RuleSyntaxError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSyntaxError(pub String);

impl std::fmt::Display for RuleSyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Number(Decimal),
    CellRef(CellRef),
    BinaryOp(char, Box<Expr>, Box<Expr>),
    If(Box<Comparison>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CellRef {
    pub cube: Option<String>,
    pub coordinates: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Comparison {
    pub op: &'static str,
    pub left: Expr,
    pub right: Expr,
}

/// A rule or feeder area: its qualifier and the element names it selects.
#[derive(Clone, Debug)]
pub struct Area {
    pub qualifier: &'static str,
    pub elements: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub area: Area,
    pub expression: Expr,
    pub source_line: usize,
}

#[derive(Clone, Debug)]
pub struct Feeder {
    pub area: Area,
    pub target: Area,
    pub source_line: usize,
    /// A feeder may point into another cube, written as an `=> DB('Cube', ...)`
    /// target, which is how TM1 feeds a reporting cube from its source cubes.
    pub target_cube: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuleSet {
    pub skipcheck: bool,
    pub rules: Vec<Rule>,
    pub feeders: Vec<Feeder>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Str,
    Number,
    Name,
    Op,
    End,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    text: String,
    line: usize,
}

fn tokenise(text: &str, path: &str) -> Result<Vec<Token>, RuleSyntaxError> {
    let chars: Vec<char> = text.chars().collect();
    let length = chars.len();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut index = 0;
    let token = |kind, text: String, line| Token { kind, text, line };
    while index < length {
        let character = chars[index];
        if character == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if matches!(character, ' ' | '\t' | '\r') {
            index += 1;
            continue;
        }
        if character == '#' {
            while index < length && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if character == '\'' {
            let mut cursor = index + 1;
            let mut parts = String::new();
            let end = loop {
                let end = (cursor..length).find(|&i| chars[i] == '\'');
                let Some(end) = end.filter(|&end| !chars[cursor..end].contains(&'\n')) else {
                    return Err(RuleSyntaxError(format!(
                        "{path} line {line}: unterminated quoted name"
                    )));
                };
                parts.extend(&chars[cursor..end]);
                if chars.get(end + 1) != Some(&'\'') {
                    break end;
                }
                parts.push('\'');
                cursor = end + 2;
            };
            tokens.push(token(Kind::Str, parts, line));
            index = end + 1;
            continue;
        }
        if py::is_digit(character)
            || (character == '.' && index + 1 < length && py::is_digit(chars[index + 1]))
        {
            let start = index;
            while index < length && (py::is_digit(chars[index]) || chars[index] == '.') {
                index += 1;
            }
            tokens.push(token(
                Kind::Number,
                chars[start..index].iter().collect(),
                line,
            ));
            continue;
        }
        if py::is_alpha(character) || character == '_' {
            let start = index;
            while index < length && (py::is_alnum(chars[index]) || chars[index] == '_') {
                index += 1;
            }
            tokens.push(token(
                Kind::Name,
                chars[start..index].iter().collect(),
                line,
            ));
            continue;
        }
        let next = chars.get(index + 1).copied();
        let pair = match (character, next) {
            ('=', Some('>')) => Some("=>"),
            ('<', Some('>')) => Some("<>"),
            ('<', Some('=')) => Some("<="),
            ('>', Some('=')) => Some(">="),
            _ => None,
        };
        if let Some(pair) = pair {
            tokens.push(token(Kind::Op, pair.into(), line));
            index += 2;
            continue;
        }
        if "[](),;=+-*/\\<>:!".contains(character) {
            tokens.push(token(Kind::Op, character.to_string(), line));
            index += 1;
            continue;
        }
        return Err(RuleSyntaxError(format!(
            "{path} line {line}: unexpected character {}",
            py::repr_str(&character.to_string())
        )));
    }
    tokens.push(token(Kind::End, String::new(), line));
    Ok(tokens)
}

/// The deepest call the Python parser reached before its interpreter raised
/// RecursionError, counting `_parse_rules` as depth 0. Measured through the
/// 3.15.0 console script, where every subcommand loads rules at the same depth.
/// Windows executes the console script's embedded ZIP through two additional
/// runpy frames; the Unix entry point is a Python file.
const MAX_DEPTH: usize = if cfg!(windows) { 990 } else { 992 };

enum Fail {
    Syntax(RuleSyntaxError),
    TooDeep,
    Uncaught(Uncaught),
}

type Parsed<T> = Result<T, Fail>;

struct Parser<'a> {
    tokens: Vec<Token>,
    path: &'a str,
    position: usize,
    depth: usize,
}

const COMPARISONS: [&str; 6] = ["=", "<", ">", "<=", ">=", "<>"];

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Str => "string",
        Kind::Number => "number",
        Kind::Name => "name",
        Kind::Op => "op",
        Kind::End => "end",
    }
}

impl Parser<'_> {
    /// A call that does not call further Python functions: a property read,
    /// a NamedTuple constructor, `Path.__str__` or `decimal_or_raise`.
    fn leaf(&self) -> Parsed<()> {
        if self.depth + 1 > MAX_DEPTH {
            Err(Fail::TooDeep)
        } else {
            Ok(())
        }
    }

    /// Enter a call that may call others.
    fn enter(&mut self) -> Parsed<()> {
        self.leaf()?;
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn call<T>(&mut self, f: impl FnOnce(&mut Self) -> Parsed<T>) -> Parsed<T> {
        self.enter()?;
        let result = f(self);
        self.leave();
        result
    }

    /// `self.current`.
    fn current(&self) -> Parsed<Token> {
        self.leaf()?;
        Ok(self.tokens[self.position].clone())
    }

    /// `self.fail(message, token)`: builds the error, or runs out of stack doing so.
    fn fail(&mut self, message: &str, token: Option<Token>) -> Parsed<Fail> {
        self.call(|p| {
            let token = match token {
                Some(token) => token,
                None => p.current()?,
            };
            p.leaf()?; // str(self.path)
            Ok(Fail::Syntax(RuleSyntaxError(format!(
                "{} line {}: {message}",
                p.path, token.line
            ))))
        })
    }

    fn advance(&mut self) -> Parsed<Token> {
        self.leaf()?;
        let token = self.tokens[self.position].clone();
        self.position += 1;
        Ok(token)
    }

    fn accept(&mut self, kind: Kind, text: Option<&str>) -> Parsed<Option<Token>> {
        self.call(|p| {
            let token = p.current()?;
            if token.kind != kind {
                return Ok(None);
            }
            if let Some(text) = text
                && py::upper(&token.text) != py::upper(text)
            {
                return Ok(None);
            }
            Ok(Some(p.advance()?))
        })
    }

    fn expect(&mut self, kind: Kind, text: Option<&str>) -> Parsed<Token> {
        self.call(|p| {
            if let Some(token) = p.accept(kind, text)? {
                return Ok(token);
            }
            let wanted = text.unwrap_or(kind_name(kind));
            let current = p.current()?;
            let found = if current.text.is_empty() {
                "end of file"
            } else {
                &current.text
            };
            let message = format!(
                "expected {} but found {}",
                py::repr_str(wanted),
                py::repr_str(found)
            );
            Err(p.fail(&message, None)?)
        })
    }

    fn parse_area(&mut self) -> Parsed<Area> {
        self.call(|p| {
            p.expect(Kind::Op, Some("["))?;
            let mut elements = Vec::new();
            if p.accept(Kind::Op, Some("]"))?.is_none() {
                loop {
                    elements.push(p.expect(Kind::Str, None)?.text);
                    if p.accept(Kind::Op, Some(","))?.is_none() {
                        break;
                    }
                }
                p.expect(Kind::Op, Some("]"))?;
            }
            p.leaf()?; // Area(...)
            Ok(Area {
                qualifier: "",
                elements,
            })
        })
    }

    fn parse_primary(&mut self) -> Parsed<Expr> {
        self.call(|p| {
            let token = p.current()?;
            if token.kind == Kind::Number {
                p.advance()?;
                p.leaf()?; // str(self.path) in the location
                p.leaf()?; // decimal_or_raise
                let value = Decimal::parse(&token.text).map_err(|_| {
                    Fail::Syntax(RuleSyntaxError(format!(
                        "{} line {}: {} is not a number",
                        p.path,
                        token.line,
                        py::repr_str(&token.text)
                    )))
                })?;
                p.leaf()?; // Number(...)
                return Ok(Expr::Number(value));
            }
            if token.kind == Kind::Op && token.text == "-" {
                p.advance()?;
                let operand = p.parse_primary()?;
                p.leaf()?;
                return Ok(match operand {
                    Expr::Number(value) => Expr::Number(
                        value
                            .neg()
                            .map_err(|signal| Fail::Uncaught(Uncaught::decimal(signal)))?,
                    ),
                    other => Expr::BinaryOp(
                        '-',
                        Box::new(Expr::Number(Decimal::zero())),
                        Box::new(other),
                    ),
                });
            }
            if token.kind == Kind::Op && token.text == "(" {
                p.advance()?;
                let expression = p.parse_expression()?;
                p.expect(Kind::Op, Some(")"))?;
                return Ok(expression);
            }
            if token.kind == Kind::Op && token.text == "[" {
                let area = p.parse_area()?;
                p.leaf()?; // the generator expression
                p.leaf()?; // CellRef(...)
                return Ok(Expr::CellRef(CellRef {
                    cube: None,
                    coordinates: area.elements,
                }));
            }
            if token.kind == Kind::Name {
                let upper = py::upper(&token.text);
                if upper == "DB" {
                    return Ok(Expr::CellRef(p.parse_db()?));
                }
                if upper == "IF" {
                    return p.parse_if();
                }
                let message = format!("{} is outside the supported rule grammar", token.text);
                return Err(p.fail(&message, None)?);
            }
            let shown = if token.text.is_empty() {
                "end of file"
            } else {
                &token.text
            };
            let message = format!("unexpected {} in an expression", py::repr_str(shown));
            Err(p.fail(&message, None)?)
        })
    }

    fn parse_db(&mut self) -> Parsed<CellRef> {
        self.call(|p| {
            p.expect(Kind::Name, Some("DB"))?;
            p.expect(Kind::Op, Some("("))?;
            let cube = p.expect(Kind::Str, None)?.text;
            let mut coordinates = Vec::new();
            while p.accept(Kind::Op, Some(","))?.is_some() {
                if p.accept(Kind::Op, Some("!"))?.is_some() {
                    let dimension = p.expect(Kind::Name, None)?;
                    coordinates.push(format!("!{}", dimension.text));
                } else {
                    coordinates.push(p.expect(Kind::Str, None)?.text);
                }
            }
            p.expect(Kind::Op, Some(")"))?;
            p.leaf()?; // CellRef(...)
            Ok(CellRef {
                cube: Some(cube),
                coordinates,
            })
        })
    }

    fn parse_if(&mut self) -> Parsed<Expr> {
        self.call(|p| {
            p.expect(Kind::Name, Some("IF"))?;
            p.expect(Kind::Op, Some("("))?;
            let condition = p.parse_comparison()?;
            p.expect(Kind::Op, Some(","))?;
            let then_expr = p.parse_expression()?;
            p.expect(Kind::Op, Some(","))?;
            let else_expr = p.parse_expression()?;
            p.expect(Kind::Op, Some(")"))?;
            p.leaf()?; // IfExpr(...)
            Ok(Expr::If(
                Box::new(condition),
                Box::new(then_expr),
                Box::new(else_expr),
            ))
        })
    }

    fn parse_comparison(&mut self) -> Parsed<Comparison> {
        self.call(|p| {
            let left = p.parse_expression()?;
            let token = p.current()?;
            let op = COMPARISONS
                .iter()
                .find(|op| token.kind == Kind::Op && token.text == **op);
            let Some(op) = op else {
                return Err(p.fail("expected a comparison operator inside IF", None)?);
            };
            p.advance()?;
            let right = p.parse_expression()?;
            p.leaf()?; // Comparison(...)
            Ok(Comparison { op, left, right })
        })
    }

    /// `self.current.kind == "op" and self.current.text in ops`, reading the
    /// property once or twice as Python's short-circuit did.
    fn at_operator(&self, ops: &[&str]) -> Parsed<Option<char>> {
        let token = self.current()?;
        if token.kind != Kind::Op {
            return Ok(None);
        }
        let token = self.current()?;
        Ok(ops
            .contains(&token.text.as_str())
            .then(|| token.text.chars().next().expect("an operator")))
    }

    fn parse_term(&mut self) -> Parsed<Expr> {
        self.call(|p| {
            let mut expression = p.parse_primary()?;
            while let Some(op) = p.at_operator(&["*", "/", "\\"])? {
                p.advance()?;
                let right = p.parse_primary()?;
                p.leaf()?; // BinaryOp(...)
                expression = Expr::BinaryOp(op, Box::new(expression), Box::new(right));
            }
            Ok(expression)
        })
    }

    fn parse_expression(&mut self) -> Parsed<Expr> {
        self.call(|p| {
            let mut expression = p.parse_term()?;
            while let Some(op) = p.at_operator(&["+", "-"])? {
                p.advance()?;
                let right = p.parse_term()?;
                p.leaf()?; // BinaryOp(...)
                expression = Expr::BinaryOp(op, Box::new(expression), Box::new(right));
            }
            Ok(expression)
        })
    }
}

/// Parse a rules file into its SKIPCHECK flag, its rules and its feeders.
pub fn parse_rules(text: &str, path: &str) -> Result<RuleSet, Error> {
    match parse_statements(text, path) {
        Ok(rules) => Ok(rules),
        Err(Fail::Syntax(error)) => Err(Error::RuleSyntax(error.0)),
        Err(Fail::TooDeep) => Err(Error::RuleSyntax(format!(
            "{path}: rules are nested too deeply to read"
        ))),
        Err(Fail::Uncaught(error)) => Err(Error::Uncaught(error)),
    }
}

fn parse_statements(text: &str, path: &str) -> Parsed<RuleSet> {
    let tokens = tokenise(text, path).map_err(Fail::Syntax)?;
    let mut parser = Parser {
        tokens,
        path,
        position: 0,
        depth: 0,
    };
    let mut skipcheck = false;
    let mut in_feeders = false;
    let mut rules = Vec::new();
    let mut feeders = Vec::new();

    while parser.current()?.kind != Kind::End {
        let token = parser.current()?;
        if token.kind == Kind::Name {
            let upper = py::upper(&token.text);
            if matches!(
                upper.as_str(),
                "SKIPCHECK" | "FEEDERS" | "UNDEFVALS" | "FEEDSTRINGS"
            ) {
                parser.advance()?;
                parser.expect(Kind::Op, Some(";"))?;
                if upper == "SKIPCHECK" {
                    skipcheck = true;
                } else if upper == "FEEDERS" {
                    in_feeders = true;
                }
                continue;
            }
            let message = format!("{} is outside the supported rule grammar", token.text);
            return Err(parser.fail(&message, Some(token))?);
        }
        if token.kind != Kind::Op || token.text != "[" {
            let shown = if token.text.is_empty() {
                "end of file"
            } else {
                &token.text
            };
            let message = format!(
                "a statement must start with an area in square brackets, found {}",
                py::repr_str(shown)
            );
            return Err(parser.fail(&message, Some(token))?);
        }
        let line = token.line;
        let area = parser.parse_area()?;
        if parser.accept(Kind::Op, Some("=>"))?.is_some() {
            let current = parser.current()?;
            let is_db = current.kind == Kind::Name && {
                let current = parser.current()?;
                py::upper(&current.text) == "DB"
            };
            let feeder = if is_db {
                let reference = parser.parse_db()?;
                parser.leaf()?; // Area(...)
                parser.leaf()?; // Feeder(...)
                Feeder {
                    area,
                    target: Area {
                        qualifier: "",
                        elements: reference.coordinates,
                    },
                    source_line: line,
                    target_cube: reference.cube,
                }
            } else {
                let target = parser.parse_area()?;
                parser.leaf()?; // Feeder(...)
                Feeder {
                    area,
                    target,
                    source_line: line,
                    target_cube: None,
                }
            };
            parser.expect(Kind::Op, Some(";"))?;
            feeders.push(feeder);
            continue;
        }
        if in_feeders {
            return Err(parser.fail(
                "a statement after FEEDERS must be a feeder using =>",
                Some(token),
            )?);
        }
        parser.expect(Kind::Op, Some("="))?;
        let mut qualifier = "";
        let current = parser.current()?;
        let qualified = current.kind == Kind::Name && {
            let current = parser.current()?;
            matches!(py::upper(&current.text).as_str(), "N" | "C")
        };
        if qualified {
            let lookahead = &parser.tokens[parser.position + 1];
            if lookahead.kind == Kind::Op && lookahead.text == ":" {
                let text = py::upper(&parser.advance()?.text);
                qualifier = if text == "N" { "N" } else { "C" };
                parser.advance()?;
            }
        }
        let expression = parser.parse_expression()?;
        parser.expect(Kind::Op, Some(";"))?;
        parser.leaf()?; // Area(...)
        parser.leaf()?; // Rule(...)
        rules.push(Rule {
            area: Area {
                qualifier,
                elements: area.elements,
            },
            expression,
            source_line: line,
        });
    }
    Ok(RuleSet {
        skipcheck,
        rules,
        feeders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> String {
        parse_rules(text, "R").unwrap_err().message()
    }

    #[test]
    fn parses_rules_and_feeders() {
        let parsed = parse_rules(
            "SKIPCHECK;\n['A'] = n: ['B'] * 2 + DB('C', !D, 'x');\nfeeders;\n['B'] => DB('C', 'y');\n",
            "R",
        )
        .unwrap();
        assert!(parsed.skipcheck);
        assert_eq!(parsed.rules[0].area.qualifier, "N");
        assert_eq!(parsed.feeders[0].target_cube.as_deref(), Some("C"));
        assert_eq!(parsed.feeders[0].source_line, 4);
    }

    #[test]
    fn errors_match_the_python_parser() {
        assert_eq!(error("['A'] = @;"), "R line 1: unexpected character '@'");
        assert_eq!(error("['A] = 1;"), "R line 1: unterminated quoted name");
        assert_eq!(
            error("['A'] = 1"),
            "R line 1: expected ';' but found 'end of file'"
        );
        assert_eq!(
            error("['A'] = ATTRS;"),
            "R line 1: ATTRS is outside the supported rule grammar"
        );
        assert_eq!(error("['A'] = 1.2.3;"), "R line 1: '1.2.3' is not a number");
        assert_eq!(
            error("FEEDERS;\n['A'] = 1;"),
            "R line 2: a statement after FEEDERS must be a feeder using =>"
        );
    }

    #[test]
    fn nesting_stops_where_python_stopped() {
        // The binary runs on a large stack; a test thread's default is too small.
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(nesting_limits)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn errors_name_the_kind_of_token_wanted() {
        assert_eq!(
            error("['A', 1] = 1;"),
            "R line 1: expected 'string' but found '1'"
        );
        assert_eq!(
            error("['A'] = DB('C', !'D');"),
            "R line 1: expected 'name' but found 'D'"
        );
        assert_eq!(
            error("['A'] = "),
            "R line 1: unexpected 'end of file' in an expression"
        );
        // An empty quoted name has empty text, which Python's `token.text or
        // "end of file"` shows as the end of the file.
        assert_eq!(
            error("'' = 1;"),
            "R line 1: a statement must start with an area in square brackets, found 'end of file'"
        );
        assert_eq!(
            RuleSyntaxError("R line 1: x".into()).to_string(),
            "R line 1: x"
        );
    }

    #[test]
    fn negating_a_number_past_emax_raises_overflow() {
        // -Decimal("1" + "0" * 1000000) has adjusted exponent 1000000, above Emax.
        let text = format!("['A'] = -1{};", "0".repeat(1_000_000));
        assert_eq!(
            parse_rules(&text, "R").unwrap_err(),
            Error::Uncaught(Uncaught::new(
                "decimal.Overflow",
                "[<class 'decimal.Overflow'>]"
            ))
        );
    }

    fn nesting_limits() {
        #[cfg(windows)]
        let (parentheses, unary, mixed) = (328, 983, 246);
        #[cfg(not(windows))]
        let (parentheses, unary, mixed) = (329, 985, 247);
        let nested = |n: usize| format!("['A'] = N: {}1{};", "(".repeat(n), ")".repeat(n));
        assert!(parse_rules(&nested(parentheses), "R").is_ok());
        assert_eq!(
            error(&nested(parentheses + 1)),
            "R: rules are nested too deeply to read"
        );
        let malformed = format!("{} +;", nested(parentheses).trim_end_matches(';'));
        assert_eq!(
            error(&malformed),
            "R line 1: unexpected ';' in an expression"
        );
        let minus = |n: usize| format!("['A'] = N: {}['B'];", "-".repeat(n));
        assert!(parse_rules(&minus(unary), "R").is_ok());
        assert!(parse_rules(&minus(unary + 1), "R").is_err());
        let ifs = |n: usize| format!("['A'] = N: {}1{};", "IF(1=1,".repeat(n), ",2)".repeat(n));
        assert!(parse_rules(&ifs(246), "R").is_ok());
        assert!(parse_rules(&ifs(247), "R").is_err());
        let mixed_expr = |n: usize| format!("['A'] = N: {}1{};", "(-".repeat(n), ")".repeat(n));
        assert!(parse_rules(&mixed_expr(mixed), "R").is_ok());
        assert!(parse_rules(&mixed_expr(mixed + 1), "R").is_err());
        let conds = |n: usize| format!("['A'] = N: {}1{};", "IF(".repeat(n), "=1,1,2)".repeat(n));
        assert!(parse_rules(&conds(197), "R").is_ok());
        assert!(parse_rules(&conds(198), "R").is_err());
    }
}

//! The command line's argument parser, as Python 3.15's `argparse` behaved for
//! this program's parsers.
//!
//! Usage lines, help text, error messages, option abbreviation, `--`
//! handling and the order in which errors are found all follow `argparse`
//! for the actions this command line declares: optionals that take one value
//! or none, one optional positional, and a required subcommand. Help is
//! wrapped to the terminal width `shutil.get_terminal_size` reports and
//! coloured where `_colorize.can_colorize` would colour it.

use std::collections::HashMap;

use crate::py;
use crate::terminal::Terminal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nargs {
    /// One value (`nargs=None`).
    One,
    /// `nargs="?"`.
    Optional,
    /// No value: help and version.
    Zero,
    /// `nargs=PARSER`: a subcommand and everything after it.
    Parser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Help,
    Version(String),
    Store,
    /// `choices=...` on a one-value optional; the value is checked as it is consumed.
    Choice(&'static [&'static str]),
    Append,
    Parsers,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub options: Vec<&'static str>,
    pub dest: &'static str,
    pub nargs: Nargs,
    pub required: bool,
    pub help: Option<&'static str>,
    pub metavar: Option<&'static str>,
    pub kind: Kind,
}

impl Action {
    pub fn help() -> Action {
        Action {
            options: vec!["-h", "--help"],
            dest: "help",
            nargs: Nargs::Zero,
            required: false,
            help: Some("show this help message and exit"),
            metavar: None,
            kind: Kind::Help,
        }
    }
}

/// A subcommand's name, its help in the parent's listing, and its parser.
pub struct Choice {
    pub name: &'static str,
    pub help: &'static str,
    pub parser: Parser,
}

pub struct Parser {
    pub prog: String,
    pub description: Option<&'static str>,
    /// Every action in the order it was added, help first.
    pub actions: Vec<Action>,
    pub choices: Vec<Choice>,
}

/// A parsed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    None,
    Str(String),
    List(Vec<String>),
}

#[derive(Debug, Default)]
pub struct Namespace {
    pub values: HashMap<&'static str, Value>,
    pub command: Option<String>,
}

impl Namespace {
    pub fn str(&self, dest: &str) -> Option<&str> {
        match self.values.get(dest) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn list(&self, dest: &str) -> &[String] {
        match self.values.get(dest) {
            Some(Value::List(items)) => items,
            _ => &[],
        }
    }
}

/// What parsing did: a namespace, or `SystemExit` after printing.
#[derive(Debug)]
pub enum Outcome {
    Parsed(Namespace),
    Exit {
        code: i32,
        stdout: String,
        stderr: String,
    },
}

struct Theme {
    usage: &'static str,
    prog: &'static str,
    heading: &'static str,
    summary_long_option: &'static str,
    summary_short_option: &'static str,
    summary_label: &'static str,
    summary_action: &'static str,
    long_option: &'static str,
    short_option: &'static str,
    label: &'static str,
    action: &'static str,
    reset: &'static str,
    error: &'static str,
    message: &'static str,
}

const COLOURLESS: Theme = Theme {
    usage: "",
    prog: "",
    heading: "",
    summary_long_option: "",
    summary_short_option: "",
    summary_label: "",
    summary_action: "",
    long_option: "",
    short_option: "",
    label: "",
    action: "",
    reset: "",
    error: "",
    message: "",
};

const COLOURED: Theme = Theme {
    usage: "\x1b[1;34m",
    prog: "\x1b[1;35m",
    heading: "\x1b[1;34m",
    summary_long_option: "\x1b[36m",
    summary_short_option: "\x1b[32m",
    summary_label: "\x1b[33m",
    summary_action: "\x1b[32m",
    long_option: "\x1b[1;36m",
    short_option: "\x1b[1;32m",
    label: "\x1b[1;33m",
    action: "\x1b[1;32m",
    reset: "\x1b[0m",
    error: "\x1b[1;35m",
    message: "\x1b[35m",
};

fn theme(colour: bool) -> &'static Theme {
    if colour { &COLOURED } else { &COLOURLESS }
}

/// `_colorize.decolor`: drop every ANSI sequence the themes use.
fn decolor(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('\x1b') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.find('m') {
            Some(end)
                if tail[1..end]
                    .chars()
                    .all(|c| c == '[' || c == ';' || c.is_ascii_digit()) =>
            {
                rest = &tail[end + 1..];
            }
            _ => {
                out.push('\x1b');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn len(text: &str) -> i64 {
    py::char_len(text) as i64
}

fn spaces(n: i64) -> String {
    " ".repeat(n.max(0) as usize)
}

/// `textwrap.wrap(text, width)` for text that holds no hyphens and no
/// whitespace except single spaces, which is all this program's help is.
// shortcut: textwrap's hyphenated-word and em-dash splitting is not ported;
// a test pins every help string the parser declares as hyphen free.
pub fn wrap(text: &str, width: i64, initial: &str, subsequent: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            chunks.push(" ".into());
        }
        if !word.is_empty() {
            chunks.push(word.to_string());
        }
    }
    chunks.reverse();
    let mut lines = Vec::new();
    while !chunks.is_empty() {
        let indent = if lines.is_empty() {
            initial
        } else {
            subsequent
        };
        let width = width - len(indent);
        if chunks.last().is_some_and(|c| c.trim().is_empty()) && !lines.is_empty() {
            chunks.pop();
        }
        let mut current: Vec<String> = Vec::new();
        let mut current_len = 0;
        while let Some(chunk) = chunks.last() {
            let l = len(chunk);
            if current_len + l <= width {
                current_len += l;
                current.push(chunks.pop().expect("a chunk"));
            } else {
                break;
            }
        }
        // _handle_long_word: split a word too long for any line. Its other
        // branch, taking the word whole onto an empty line, is never reached
        // with break_long_words on: an empty line leaves space_left > 0.
        let space_left = if width < 1 { 1 } else { width - current_len };
        if space_left > 0
            && let Some(chunk) = chunks.pop_if(|c| len(c) > width)
        {
            let split = py::prefix_chars(&chunk, space_left as usize).to_string();
            chunks.push(chunk[split.len()..].to_string());
            current.push(split);
        }
        if current.last().is_some_and(|c| c.trim().is_empty()) {
            current.pop();
        }
        if !current.is_empty() {
            lines.push(format!("{indent}{}", current.concat()));
        }
    }
    lines
}

/// `re.sub(r"\s+", " ", text, flags=re.ASCII).strip()`.
fn normalise_whitespace(text: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') {
            in_space = true;
        } else {
            if in_space && !out.is_empty() {
                out.push(' ');
            }
            in_space = false;
            out.push(c);
        }
    }
    out
}

struct Formatter<'a> {
    prog: &'a str,
    width: i64,
    max_help_position: i64,
    indent: i64,
    action_max_length: i64,
    t: &'static Theme,
}

impl Formatter<'_> {
    fn new(prog: &str, columns: i64, colour: bool) -> Formatter<'_> {
        let width = columns - 2;
        Formatter {
            prog,
            width,
            max_help_position: 24.min((width - 20).max(4)),
            indent: 0,
            action_max_length: 0,
            t: theme(colour),
        }
    }

    fn format_args(action: &Action, default: &str) -> String {
        let metavar = action.metavar.unwrap_or(default);
        match action.nargs {
            Nargs::One => metavar.to_string(),
            Nargs::Optional => format!("[{metavar}]"),
            Nargs::Parser => format!("{metavar} ..."),
            Nargs::Zero => String::new(),
        }
    }

    fn default_metavar(action: &Action) -> String {
        if let Kind::Choice(choices) = &action.kind {
            format!("{{{}}}", choices.join(","))
        } else if action.options.is_empty() {
            action.dest.to_string()
        } else {
            action.dest.to_uppercase()
        }
    }

    fn usage_parts(&self, actions: &[Action]) -> (Vec<String>, usize) {
        let t = self.t;
        let mut parts = Vec::new();
        for action in actions.iter().filter(|a| !a.options.is_empty()) {
            let option = action.options[0];
            let colour = if option.len() > 2 {
                t.summary_long_option
            } else {
                t.summary_short_option
            };
            let mut part = if action.nargs == Nargs::Zero {
                format!("{colour}{option}{}", t.reset)
            } else {
                let args = Self::format_args(action, &Self::default_metavar(action));
                format!("{colour}{option} {}{args}{}", t.summary_label, t.reset)
            };
            if !action.required {
                part = format!("[{part}]");
            }
            parts.push(part);
        }
        let pos_start = parts.len();
        for action in actions.iter().filter(|a| a.options.is_empty()) {
            let part = Self::format_args(action, &Self::default_metavar(action));
            parts.push(format!("{}{part}{}", t.summary_action, t.reset));
        }
        (parts, pos_start)
    }

    fn format_usage(&self, actions: &[Action]) -> String {
        let t = self.t;
        let prefix = "usage: ";
        let prog = self.prog.to_string();
        let (parts, pos_start) = self.usage_parts(actions);
        let mut all = vec![prog.clone()];
        all.extend(parts.iter().cloned());
        let mut usage = all.join(" ");
        let text_width = self.width - self.indent;
        if len(prefix) + len(&decolor(&usage)) > text_width {
            let opt_parts = &parts[..pos_start];
            let pos_parts = &parts[pos_start..];
            let get_lines = |parts: &[String], indent: &str, prefix: Option<&str>| {
                let mut lines: Vec<String> = Vec::new();
                let mut line: Vec<&str> = Vec::new();
                let indent_length = len(indent);
                let mut line_len = match prefix {
                    Some(p) => len(p) - 1,
                    None => indent_length - 1,
                };
                for part in parts {
                    let part_len = len(&decolor(part));
                    if line_len + 1 + part_len > text_width && !line.is_empty() {
                        lines.push(format!("{indent}{}", line.join(" ")));
                        line.clear();
                        line_len = indent_length - 1;
                    }
                    line.push(part);
                    line_len += part_len + 1;
                }
                if !line.is_empty() {
                    lines.push(format!("{indent}{}", line.join(" ")));
                }
                if prefix.is_some() {
                    lines[0] = lines[0][indent.len()..].to_string();
                }
                lines
            };
            let prog_len = len(&decolor(&prog));
            let lines = if 4 * (len(prefix) + prog_len) <= 3 * text_width {
                let indent = spaces(len(prefix) + prog_len + 1);
                let mut with_prog = vec![prog.clone()];
                if !opt_parts.is_empty() {
                    with_prog.extend(opt_parts.iter().cloned());
                    let mut lines = get_lines(&with_prog, &indent, Some(prefix));
                    lines.extend(get_lines(pos_parts, &indent, None));
                    lines
                } else {
                    // argparse has a separate `[prog]` branch for no parts at
                    // all, which this gives too. It cannot be reached: a usage
                    // of prog alone that is too wide fails the test above.
                    with_prog.extend(pos_parts.iter().cloned());
                    get_lines(&with_prog, &indent, Some(prefix))
                }
            } else {
                let indent = spaces(len(prefix));
                let mut lines = get_lines(&parts, &indent, None);
                if lines.len() > 1 {
                    lines = get_lines(opt_parts, &indent, None);
                    lines.extend(get_lines(pos_parts, &indent, None));
                }
                let mut out = vec![prog.clone()];
                out.extend(lines);
                out
            };
            usage = lines.join("\n");
        }
        let usage = usage.strip_prefix(&prog).unwrap_or(&usage).to_string();
        format!(
            "{}{prefix}{}{}{prog}{}{usage}\n\n",
            t.usage, t.reset, t.prog, t.reset
        )
    }

    fn format_text(&self, text: &str) -> String {
        let text_width = (self.width - self.indent).max(11);
        let indent = spaces(self.indent);
        let lines = wrap(&normalise_whitespace(text), text_width, &indent, &indent);
        format!("{}\n\n", lines.join("\n"))
    }

    fn invocation(&self, action: &Action) -> String {
        let t = self.t;
        if action.options.is_empty() {
            let metavar = action.metavar.unwrap_or(action.dest);
            return format!("{}{metavar}{}", t.action, t.reset);
        }
        let options: Vec<String> = action
            .options
            .iter()
            .map(|s| {
                let colour = if s.len() > 2 {
                    t.long_option
                } else {
                    t.short_option
                };
                format!("{colour}{s}{}", t.reset)
            })
            .collect();
        if action.nargs == Nargs::Zero {
            return options.join(", ");
        }
        let args = Self::format_args(action, &Self::default_metavar(action));
        format!("{} {}{args}{}", options.join(", "), t.label, t.reset)
    }

    fn format_action(&self, action: &Action, indent: i64, subactions: &[Action]) -> String {
        let help_position = (self.action_max_length + 2).min(self.max_help_position);
        let help_width = (self.width - help_position).max(11);
        let action_width = help_position - indent - 2;
        let header = self.invocation(action);
        let plain = decolor(&header);
        let mut parts = Vec::new();
        let help = action.help.unwrap_or("");
        let indent_first = if help.is_empty() {
            parts.push(format!("{}{header}\n", spaces(indent)));
            0
        } else if len(&plain) <= action_width {
            let padded = format!(
                "{}{plain}{}  ",
                spaces(indent),
                spaces(action_width - len(&plain))
            );
            parts.push(padded.replacen(&plain, &header, 1));
            0
        } else {
            parts.push(format!("{}{header}\n", spaces(indent)));
            help_position
        };
        if !help.trim().is_empty() {
            let lines = wrap(&normalise_whitespace(help), help_width, "", "");
            parts.push(format!("{}{}\n", spaces(indent_first), lines[0]));
            for line in &lines[1..] {
                parts.push(format!("{}{line}\n", spaces(help_position)));
            }
        } else if !parts[0].ends_with('\n') {
            parts.push("\n".into());
        }
        for sub in subactions {
            parts.push(self.format_action(sub, indent + 2, &[]));
        }
        parts.concat()
    }
}

/// The pseudo-actions that list a parser's subcommands in its help.
fn choice_actions(parser: &Parser) -> Vec<Action> {
    parser
        .choices
        .iter()
        .map(|c| Action {
            options: vec![],
            dest: c.name,
            nargs: Nargs::One,
            required: false,
            help: Some(c.help),
            metavar: Some(c.name),
            kind: Kind::Store,
        })
        .collect()
}

impl Parser {
    fn format_usage(&self, terminal: &Terminal, colour: bool) -> String {
        let formatter = Formatter::new(&self.prog, terminal.columns, colour);
        let text = formatter.format_usage(&self.actions);
        finish(&text)
    }

    pub fn format_help(&self, terminal: &Terminal, colour: bool) -> String {
        let mut formatter = Formatter::new(&self.prog, terminal.columns, colour);
        let subactions = choice_actions(self);
        let positionals: Vec<&Action> = self
            .actions
            .iter()
            .filter(|a| a.options.is_empty())
            .collect();
        let optionals: Vec<&Action> = self
            .actions
            .iter()
            .filter(|a| !a.options.is_empty())
            .collect();
        // add_argument measures every invocation before anything is formatted.
        for action in positionals.iter().chain(&optionals) {
            let mut lengths = vec![len(&decolor(&formatter.invocation(action))) + 2];
            if action.kind == Kind::Parsers {
                for sub in &subactions {
                    lengths.push(len(&decolor(&formatter.invocation(sub))) + 4);
                }
            }
            formatter.action_max_length = formatter
                .action_max_length
                .max(*lengths.iter().max().expect("a length"));
        }
        let mut out = formatter.format_usage(&self.actions);
        if let Some(description) = self.description {
            out.push_str(&formatter.format_text(description));
        }
        for (heading, actions) in [
            ("positional arguments", &positionals),
            ("options", &optionals),
        ] {
            formatter.indent = 2;
            let items: String = actions
                .iter()
                .map(|a| {
                    let subs: &[Action] = if a.kind == Kind::Parsers {
                        &subactions
                    } else {
                        &[]
                    };
                    formatter.format_action(a, 2, subs)
                })
                .collect();
            formatter.indent = 0;
            if !items.is_empty() {
                out.push_str(&format!(
                    "\n{}{heading}:{}\n{items}\n",
                    formatter.t.heading, formatter.t.reset
                ));
            }
        }
        finish(&out)
    }

    fn error(&self, message: &str, terminal: &Terminal) -> Outcome {
        let colour = terminal.colour_stderr;
        let t = theme(colour);
        let mut stderr = self.format_usage(terminal, colour);
        stderr.push_str(&format!(
            "{}: {}error:{} {}{message}{}\n",
            self.prog, t.error, t.reset, t.message, t.reset
        ));
        Outcome::Exit {
            code: 2,
            stdout: String::new(),
            stderr,
        }
    }

    fn option_actions(&self) -> Vec<(&'static str, usize)> {
        let mut out = Vec::new();
        for (index, action) in self.actions.iter().enumerate() {
            for option in &action.options {
                out.push((*option, index));
            }
        }
        out
    }
}

/// `format_help`'s last steps: collapse runs of blank lines, one newline at the end.
fn finish(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            continue;
        }
        if newlines > 0 {
            collapsed.push_str(if newlines >= 3 {
                "\n\n"
            } else {
                &"\n\n"[..newlines]
            });
            newlines = 0;
        }
        collapsed.push(c);
    }
    if newlines > 0 {
        collapsed.push_str(if newlines >= 3 {
            "\n\n"
        } else {
            &"\n\n"[..newlines]
        });
    }
    let trimmed = collapsed.trim_matches('\n');
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{trimmed}\n")
}

/// One way to read an option string: the action it names (if any), the
/// option string matched, the separator and the explicit argument.
#[derive(Clone, Debug)]
struct OptionTuple {
    action: Option<usize>,
    option: String,
    sep: Option<String>,
    explicit: Option<String>,
}

/// An `ArgumentError`, or a printing exit from a help or version action.
enum Stop {
    Error(String),
    Exit(Outcome),
}

fn action_name(action: &Action) -> String {
    if !action.options.is_empty() {
        action.options.join("/")
    } else {
        action.metavar.unwrap_or(action.dest).to_string()
    }
}

/// `_check_value`'s message, with the close-match suggestion `suggest_on_error` adds.
fn invalid_choice(value: &str, names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| py::repr_str(n)).collect();
    match close_match(value, names) {
        Some(closest) => format!(
            "invalid choice: {}, maybe you meant {}? (choose from {})",
            py::repr_str(value),
            py::repr_str(closest),
            quoted.join(", ")
        ),
        None => format!(
            "invalid choice: {} (choose from {})",
            py::repr_str(value),
            quoted.join(", ")
        ),
    }
}

fn argument_error(action: &Action, message: &str) -> Stop {
    Stop::Error(format!("argument {}: {message}", action_name(action)))
}

impl Parser {
    fn parse_optional(&self, arg: &str) -> Result<Option<Vec<OptionTuple>>, Stop> {
        if arg.is_empty() || !arg.starts_with('-') {
            return Ok(None);
        }
        let options = self.option_actions();
        let exact = |s: &str| options.iter().find(|(o, _)| *o == s).map(|(_, i)| *i);
        if let Some(index) = exact(arg) {
            return Ok(Some(vec![OptionTuple {
                action: Some(index),
                option: arg.into(),
                sep: None,
                explicit: None,
            }]));
        }
        if py::char_len(arg) == 1 {
            return Ok(None);
        }
        if let Some((option, explicit)) = arg.split_once('=')
            && let Some(index) = exact(option)
        {
            return Ok(Some(vec![OptionTuple {
                action: Some(index),
                option: option.into(),
                sep: Some("=".into()),
                explicit: Some(explicit.into()),
            }]));
        }
        let tuples = self.option_tuples(arg);
        if !tuples.is_empty() {
            return Ok(Some(tuples));
        }
        let mut chars = arg.chars();
        chars.next();
        // The argument has at least two characters here.
        let negative = match chars.next() {
            Some('.') => chars.next().is_some_and(py::is_decimal),
            second => second.is_some_and(py::is_decimal),
        };
        if negative {
            return Ok(None);
        }
        if arg.contains(' ') {
            return Ok(None);
        }
        Ok(Some(vec![OptionTuple {
            action: None,
            option: arg.into(),
            sep: None,
            explicit: None,
        }]))
    }

    fn option_tuples(&self, arg: &str) -> Vec<OptionTuple> {
        let options = self.option_actions();
        let mut result = Vec::new();
        let (prefix, sep, explicit) = match arg.split_once('=') {
            Some((p, e)) => (p, Some("=".to_string()), Some(e.to_string())),
            None => (arg, None, None),
        };
        let second = arg.chars().nth(1);
        if second == Some('-') {
            for (option, index) in &options {
                if option.starts_with(prefix) {
                    result.push(OptionTuple {
                        action: Some(*index),
                        option: option.to_string(),
                        sep: sep.clone(),
                        explicit: explicit.clone(),
                    });
                }
            }
        } else {
            let short = py::prefix_chars(arg, 2);
            let short_explicit = arg[short.len()..].to_string();
            for (option, index) in &options {
                if *option == short {
                    result.push(OptionTuple {
                        action: Some(*index),
                        option: option.to_string(),
                        sep: Some(String::new()),
                        explicit: Some(short_explicit.clone()),
                    });
                } else if option.starts_with(prefix) {
                    result.push(OptionTuple {
                        action: Some(*index),
                        option: option.to_string(),
                        sep: sep.clone(),
                        explicit: explicit.clone(),
                    });
                }
            }
        }
        result
    }

    /// `parse_known_args`: the namespace and any arguments left over.
    fn parse_known(
        &self,
        args: &[String],
        terminal: &Terminal,
    ) -> Result<(Namespace, Vec<String>), Stop> {
        let mut namespace = Namespace::default();
        for action in &self.actions {
            match action.kind {
                Kind::Store | Kind::Append => {
                    namespace.values.insert(action.dest, Value::None);
                }
                _ => {}
            }
        }
        let mut option_indices: HashMap<usize, Vec<OptionTuple>> = HashMap::new();
        let mut pattern: Vec<char> = Vec::new();
        let mut iter = args.iter().enumerate();
        while let Some((i, arg)) = iter.next() {
            if arg == "--" {
                pattern.push('-');
                pattern.extend(iter.by_ref().map(|_| 'A'));
                break;
            }
            match self.parse_optional(arg)? {
                None => pattern.push('A'),
                Some(tuples) => {
                    option_indices.insert(i, tuples);
                    pattern.push('O');
                }
            }
        }

        let mut seen: Vec<usize> = Vec::new();
        let mut extras: Vec<String> = Vec::new();
        let mut unrecognised: Vec<String> = Vec::new();
        let positionals: Vec<usize> = (0..self.actions.len())
            .filter(|&i| self.actions[i].options.is_empty())
            .collect();
        let mut remaining_positionals = positionals.as_slice();

        let max_option = option_indices.keys().copied().max();
        let mut start = 0usize;
        if let Some(max_option) = max_option {
            while start <= max_option {
                let mut next = start;
                while next <= max_option && !option_indices.contains_key(&next) {
                    next += 1;
                }
                if start != next {
                    let end = self.consume_positionals(
                        start,
                        args,
                        &pattern,
                        &mut remaining_positionals,
                        &mut namespace,
                        &mut seen,
                        &mut unrecognised,
                        terminal,
                    )?;
                    if end > start {
                        start = end;
                        continue;
                    }
                    start = end;
                }
                if !option_indices.contains_key(&start) {
                    extras.extend(args[start..next].iter().cloned());
                    start = next;
                }
                start = self.consume_optional(
                    start,
                    args,
                    &pattern,
                    &option_indices,
                    &mut namespace,
                    &mut seen,
                    &mut extras,
                    terminal,
                )?;
            }
        }
        let stop = self.consume_positionals(
            start,
            args,
            &pattern,
            &mut remaining_positionals,
            &mut namespace,
            &mut seen,
            &mut unrecognised,
            terminal,
        )?;
        extras.extend(args[stop..].iter().cloned());

        let required: Vec<String> = self
            .actions
            .iter()
            .enumerate()
            .filter(|(i, a)| !seen.contains(i) && a.required)
            .map(|(_, a)| action_name(a))
            .collect();
        if !required.is_empty() {
            return Err(Stop::Error(format!(
                "the following arguments are required: {}",
                required.join(", ")
            )));
        }
        extras.extend(unrecognised);
        Ok((namespace, extras))
    }

    fn nargs_pattern(action: &Action) -> &'static str {
        let option = !action.options.is_empty();
        match (action.nargs, option) {
            (Nargs::One, true) => "A",
            (Nargs::One, false) => "-*A-*",
            (Nargs::Optional, true) => "A?",
            (Nargs::Optional, false) => "-*A?-*",
            (Nargs::Parser, true) => "A[AO]*",
            (Nargs::Parser, false) => "-*A[-AO]*",
            (Nargs::Zero, _) => "",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn consume_positionals(
        &self,
        start: usize,
        args: &[String],
        pattern: &[char],
        positionals: &mut &[usize],
        namespace: &mut Namespace,
        seen: &mut Vec<usize>,
        unrecognised: &mut Vec<String>,
        terminal: &Terminal,
    ) -> Result<usize, Stop> {
        let selected = &pattern[start.min(pattern.len())..];
        let counts = match_partial(
            &positionals
                .iter()
                .map(|&i| Self::nargs_pattern(&self.actions[i]))
                .collect::<Vec<_>>(),
            selected,
        );
        let mut index = start;
        for (&action_index, &count) in positionals.iter().zip(&counts) {
            let action = &self.actions[action_index];
            let mut taken: Vec<String> = args[index..index + count].to_vec();
            if action.nargs == Nargs::Parser {
                if pattern.get(index) == Some(&'-') {
                    taken.remove(0);
                }
            } else if pattern[index..index + count].contains(&'-')
                && let Some(at) = taken.iter().position(|a| a == "--")
            {
                taken.remove(at);
            }
            index += count;
            self.take_action(action_index, taken, namespace, seen, unrecognised, terminal)?;
        }
        *positionals = &positionals[counts.len()..];
        Ok(index)
    }

    #[allow(clippy::too_many_arguments)]
    fn consume_optional(
        &self,
        start: usize,
        args: &[String],
        pattern: &[char],
        option_indices: &HashMap<usize, Vec<OptionTuple>>,
        namespace: &mut Namespace,
        seen: &mut Vec<usize>,
        extras: &mut Vec<String>,
        terminal: &Terminal,
    ) -> Result<usize, Stop> {
        let tuples = &option_indices[&start];
        if tuples.len() > 1 {
            let matches: Vec<&str> = tuples.iter().map(|t| t.option.as_str()).collect();
            return Err(Stop::Error(format!(
                "ambiguous option: {} could match {}",
                args[start],
                matches.join(", ")
            )));
        }
        let OptionTuple {
            mut action,
            mut option,
            mut sep,
            mut explicit,
        } = tuples[0].clone();
        let mut action_tuples: Vec<(usize, Vec<String>)> = Vec::new();
        let stop;
        loop {
            let Some(index) = action else {
                extras.push(args[start].clone());
                return Ok(start + 1);
            };
            let current = &self.actions[index];
            if let Some(explicit_arg) = explicit.clone() {
                let arg_count = if current.nargs == Nargs::Zero { 0 } else { 1 };
                let second = option.chars().nth(1);
                if arg_count == 0 && second != Some('-') && !explicit_arg.is_empty() {
                    if sep.as_deref().is_some_and(|s| !s.is_empty())
                        || explicit_arg.starts_with('-')
                    {
                        return Err(argument_error(
                            current,
                            &format!("ignored explicit argument {}", py::repr_str(&explicit_arg)),
                        ));
                    }
                    action_tuples.push((index, Vec::new()));
                    let first = explicit_arg.chars().next().expect("a character");
                    option = format!("-{first}");
                    let options = self.option_actions();
                    match options.iter().find(|(o, _)| **o == option) {
                        Some((_, next)) => {
                            action = Some(*next);
                            let rest = &explicit_arg[first.len_utf8()..];
                            if rest.is_empty() {
                                sep = None;
                                explicit = None;
                            } else if let Some(after) = rest.strip_prefix('=') {
                                sep = Some("=".into());
                                explicit = Some(after.into());
                            } else {
                                sep = Some(String::new());
                                explicit = Some(rest.into());
                            }
                        }
                        None => {
                            extras.push(format!("-{explicit_arg}"));
                            stop = start + 1;
                            break;
                        }
                    }
                } else if arg_count == 1 {
                    stop = start + 1;
                    action_tuples.push((index, vec![explicit_arg]));
                    break;
                } else {
                    return Err(argument_error(
                        current,
                        &format!("ignored explicit argument {}", py::repr_str(&explicit_arg)),
                    ));
                }
            } else {
                let next = start + 1;
                let count = match current.nargs {
                    Nargs::Zero => 0,
                    Nargs::One => {
                        if pattern.get(next) == Some(&'A') {
                            1
                        } else {
                            return Err(argument_error(current, "expected one argument"));
                        }
                    }
                    Nargs::Optional => usize::from(pattern.get(next) == Some(&'A')),
                    Nargs::Parser => unreachable!("no optional takes a subcommand"),
                };
                stop = next + count;
                action_tuples.push((index, args[next..stop].to_vec()));
                break;
            }
        }
        for (index, values) in action_tuples {
            self.take_action(index, values, namespace, seen, extras, terminal)?;
        }
        Ok(stop)
    }

    fn take_action(
        &self,
        index: usize,
        values: Vec<String>,
        namespace: &mut Namespace,
        seen: &mut Vec<usize>,
        unrecognised: &mut Vec<String>,
        terminal: &Terminal,
    ) -> Result<(), Stop> {
        seen.push(index);
        let action = &self.actions[index];
        match &action.kind {
            Kind::Help => Err(Stop::Exit(Outcome::Exit {
                code: 0,
                stdout: self.format_help(terminal, terminal.colour_stdout),
                stderr: String::new(),
            })),
            Kind::Version(version) => {
                let formatter =
                    Formatter::new(&self.prog, terminal.columns, terminal.colour_stdout);
                Err(Stop::Exit(Outcome::Exit {
                    code: 0,
                    stdout: finish(&formatter.format_text(version)),
                    stderr: String::new(),
                }))
            }
            Kind::Store => {
                // One value is stored as given; `nargs="?"` with none stores None.
                let value = match values.as_slice() {
                    [one] => Value::Str(one.clone()),
                    _ => Value::None,
                };
                namespace.values.insert(action.dest, value);
                Ok(())
            }
            Kind::Append => {
                let mut items = match namespace.values.remove(action.dest) {
                    Some(Value::List(items)) => items,
                    _ => Vec::new(),
                };
                items.extend(values);
                namespace.values.insert(action.dest, Value::List(items));
                Ok(())
            }
            Kind::Choice(choices) => {
                let value = &values[0];
                if !choices.contains(&value.as_str()) {
                    return Err(argument_error(action, &invalid_choice(value, choices)));
                }
                namespace
                    .values
                    .insert(action.dest, Value::Str(value.clone()));
                Ok(())
            }
            Kind::Parsers => {
                let name = &values[0];
                let Some(choice) = self.choices.iter().find(|c| c.name == name.as_str()) else {
                    let names: Vec<&str> = self.choices.iter().map(|c| c.name).collect();
                    return Err(argument_error(action, &invalid_choice(name, &names)));
                };
                namespace.command = Some(name.clone());
                match choice.parser.parse_known(&values[1..], terminal) {
                    Ok((sub, extras)) => {
                        namespace.values.extend(sub.values);
                        unrecognised.extend(extras);
                        Ok(())
                    }
                    Err(Stop::Error(message)) => {
                        Err(Stop::Exit(choice.parser.error(&message, terminal)))
                    }
                    Err(exit) => Err(exit),
                }
            }
        }
    }

    /// `parse_args(argv)`.
    pub fn parse_args(&self, args: &[String], terminal: &Terminal) -> Outcome {
        match self.parse_known(args, terminal) {
            Ok((namespace, extras)) if extras.is_empty() => Outcome::Parsed(namespace),
            Ok((_, extras)) => self.error(
                &format!("unrecognized arguments: {}", extras.join(" ")),
                terminal,
            ),
            Err(Stop::Error(message)) => self.error(&message, terminal),
            Err(Stop::Exit(outcome)) => outcome,
        }
    }
}

/// `_match_arguments_partial`: how many strings each leading positional takes.
fn match_partial(patterns: &[&str], selected: &[char]) -> Vec<usize> {
    for count in (1..=patterns.len()).rev() {
        if let Some((counts, end)) = match_sequence(&patterns[..count], selected) {
            let mut result = counts;
            if end < selected.len() && selected[end] == 'O' {
                while result.last() == Some(&0) {
                    result.pop();
                }
            }
            return result;
        }
    }
    Vec::new()
}

/// One pattern element: the characters it accepts, whether it is optional and
/// whether it repeats.
type Element = (Vec<char>, bool, bool);

/// `re.match` of concatenated nargs patterns: greedy, backtracking, anchored at the start.
fn match_sequence(patterns: &[&str], s: &[char]) -> Option<(Vec<usize>, usize)> {
    fn tokens(pattern: &str) -> Vec<Element> {
        // (characters accepted, optional, repeated)
        let mut out = Vec::new();
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let set = if chars[i] == '[' {
                let end = chars[i..].iter().position(|&c| c == ']').expect("a class") + i;
                let set = chars[i + 1..end].to_vec();
                i = end + 1;
                set
            } else {
                i += 1;
                vec![chars[i - 1]]
            };
            let (optional, repeated) = match chars.get(i) {
                Some('*') => {
                    i += 1;
                    (true, true)
                }
                Some('?') => {
                    i += 1;
                    (true, false)
                }
                _ => (false, false),
            };
            out.push((set, optional, repeated));
        }
        out
    }
    let groups: Vec<Vec<Element>> = patterns.iter().map(|p| tokens(p)).collect();
    let flat: Vec<(usize, &Element)> = groups
        .iter()
        .enumerate()
        .flat_map(|(g, ts)| ts.iter().map(move |t| (g, t)))
        .collect();
    fn search(
        flat: &[(usize, &Element)],
        s: &[char],
        at: usize,
        pos: usize,
        taken: &mut Vec<usize>,
    ) -> Option<usize> {
        if at == flat.len() {
            return Some(pos);
        }
        let (_, (set, optional, repeated)) = flat[at];
        let mut max = 0;
        while pos + max < s.len() && set.contains(&s[pos + max]) && (*repeated || max < 1) {
            max += 1;
        }
        let min = usize::from(!*optional);
        let mut n = max;
        loop {
            if n < min {
                return None;
            }
            taken.push(n);
            if let Some(end) = search(flat, s, at + 1, pos + n, taken) {
                return Some(end);
            }
            taken.pop();
            if n == 0 {
                return None;
            }
            n -= 1;
        }
    }
    let mut taken = Vec::new();
    let end = search(&flat, s, 0, 0, &mut taken)?;
    let mut counts = vec![0; patterns.len()];
    for ((group, _), n) in flat.iter().zip(&taken) {
        counts[*group] += n;
    }
    Some((counts, end))
}

/// `difflib.get_close_matches(word, possibilities, n=1)`.
pub fn close_match<'a>(word: &str, possibilities: &[&'a str]) -> Option<&'a str> {
    let b: Vec<char> = word.chars().collect();
    let mut best: Option<(f64, &str)> = None;
    for x in possibilities {
        let a: Vec<char> = x.chars().collect();
        let matcher = SequenceMatcher::new(&a, &b);
        if matcher.real_quick_ratio() >= 0.6 && matcher.quick_ratio() >= 0.6 {
            let ratio = matcher.ratio();
            if ratio >= 0.6 {
                let better = match best {
                    None => true,
                    Some((score, name)) => ratio > score || (ratio == score && *x > name),
                };
                if better {
                    best = Some((ratio, x));
                }
            }
        }
    }
    best.map(|(_, x)| x)
}

/// A port of `difflib.SequenceMatcher` with its default autojunk.
struct SequenceMatcher<'a> {
    a: &'a [char],
    b: &'a [char],
    b2j: HashMap<char, Vec<usize>>,
}

impl<'a> SequenceMatcher<'a> {
    fn new(a: &'a [char], b: &'a [char]) -> SequenceMatcher<'a> {
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (i, c) in b.iter().enumerate() {
            b2j.entry(*c).or_default().push(i);
        }
        let n = b.len();
        if n >= 200 {
            let ntest = n / 100 + 1;
            b2j.retain(|_, indices| indices.len() <= ntest);
        }
        SequenceMatcher { a, b, b2j }
    }

    fn find_longest_match(
        &self,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let (a, b) = (self.a, self.b);
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, c) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut newj2len = HashMap::new();
            if let Some(indices) = self.b2j.get(c) {
                for &j in indices {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j
                        .checked_sub(1)
                        .and_then(|p| j2len.get(&p))
                        .copied()
                        .unwrap_or(0)
                        + 1;
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = newj2len;
        }
        // No junk is defined, so only popular elements were removed from b2j.
        let popular = |c: &char| !self.b2j.contains_key(c);
        while besti > alo && bestj > blo && !popular(&b[bestj - 1]) && a[besti - 1] == b[bestj - 1]
        {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi
            && bestj + bestsize < bhi
            && !popular(&b[bestj + bestsize])
            && a[besti + bestsize] == b[bestj + bestsize]
        {
            bestsize += 1;
        }
        while besti > alo && bestj > blo && popular(&b[bestj - 1]) && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi
            && bestj + bestsize < bhi
            && popular(&b[bestj + bestsize])
            && a[besti + bestsize] == b[bestj + bestsize]
        {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    fn matches(&self) -> usize {
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        let mut total = 0;
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.find_longest_match(alo, ahi, blo, bhi);
            if k > 0 {
                total += k;
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        total
    }

    fn ratio_of(matches: usize, length: usize) -> f64 {
        if length == 0 {
            1.0
        } else {
            2.0 * matches as f64 / length as f64
        }
    }

    fn ratio(&self) -> f64 {
        Self::ratio_of(self.matches(), self.a.len() + self.b.len())
    }

    fn quick_ratio(&self) -> f64 {
        let mut full: HashMap<char, i64> = HashMap::new();
        for c in self.b {
            *full.entry(*c).or_default() += 1;
        }
        let mut avail: HashMap<char, i64> = HashMap::new();
        let mut matches = 0;
        for c in self.a {
            let n = *avail.get(c).unwrap_or_else(|| full.get(c).unwrap_or(&0));
            avail.insert(*c, n - 1);
            if n > 0 {
                matches += 1;
            }
        }
        Self::ratio_of(matches, self.a.len() + self.b.len())
    }

    fn real_quick_ratio(&self) -> f64 {
        let (la, lb) = (self.a.len(), self.b.len());
        Self::ratio_of(la.min(lb), la + lb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_matches_follow_difflib() {
        let names = ["validate", "compare", "calculate", "explain", "report"];
        assert_eq!(close_match("valdate", &names), Some("validate"));
        assert_eq!(close_match("calc", &names), Some("calculate"));
        assert_eq!(close_match("reprot", &names), Some("report"));
        assert_eq!(close_match("", &names), None);
    }

    #[test]
    fn partial_matching_follows_argparse() {
        let a: Vec<char> = "AOA".chars().collect();
        assert_eq!(match_partial(&["-*A?-*"], &a), vec![1]);
        let b: Vec<char> = "OA".chars().collect();
        assert_eq!(match_partial(&["-*A?-*"], &b), Vec::<usize>::new());
        let c: Vec<char> = "-AOA".chars().collect();
        assert_eq!(match_partial(&["-*A[-AO]*"], &c), vec![4]);
        assert_eq!(match_partial(&["-*A[-AO]*"], &b), Vec::<usize>::new());
    }

    #[test]
    fn wrapping_follows_textwrap() {
        assert_eq!(
            wrap(
                "compare two data snapshots using the same model and requested cells",
                44,
                "",
                ""
            ),
            vec![
                "compare two data snapshots using the same",
                "model and requested cells"
            ]
        );
        assert_eq!(wrap("abcdefghij", 4, "", ""), vec!["abcd", "efgh", "ij"]);
    }
    fn terminal() -> Terminal {
        Terminal {
            columns: 80,
            stdout_tty: false,
            colour_stdout: false,
            colour_stderr: false,
        }
    }

    #[test]
    fn namespace_accessors_fall_back_for_absent_or_other_shapes() {
        let mut ns = Namespace::default();
        ns.values
            .insert("items", Value::List(vec!["a".to_string()]));
        ns.values.insert("text", Value::Str("v".into()));
        assert_eq!(ns.list("items"), &["a".to_string()]);
        assert!(ns.list("absent").is_empty());
        assert!(ns.list("text").is_empty());
        assert_eq!(ns.str("text"), Some("v"));
        assert_eq!(ns.str("items"), None);
        assert_eq!(ns.str("absent"), None);
    }

    #[test]
    fn wrap_honours_initial_and_subsequent_indents() {
        assert_eq!(wrap("abcdef", 4, "xx", ""), vec!["xxab", "cdef"]);
        assert_eq!(wrap("abcdefgh", 4, "", "xx"), vec!["abcd", "xxef", "xxgh"]);
    }

    #[test]
    fn help_renders_every_nargs_usage_shape() {
        let parser = Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![
                Action::help(),
                Action {
                    options: vec!["--one"],
                    dest: "one",
                    nargs: Nargs::One,
                    required: false,
                    help: Some("one value"),
                    metavar: Some("VALUE"),
                    kind: Kind::Store,
                },
                Action {
                    options: vec!["--maybe"],
                    dest: "maybe",
                    nargs: Nargs::Optional,
                    required: false,
                    help: Some("maybe a value"),
                    metavar: Some("VALUE"),
                    kind: Kind::Store,
                },
                Action {
                    options: vec!["--rest"],
                    dest: "rest",
                    nargs: Nargs::Parser,
                    required: false,
                    help: Some("the rest"),
                    metavar: Some("VALUE"),
                    kind: Kind::Append,
                },
            ],
            choices: vec![],
        };
        let help = parser.format_help(&terminal(), false);
        assert!(help.contains("usage: planning"), "{help}");
        assert!(help.contains("[--maybe [VALUE]]"), "{help}");
        assert!(help.contains("[--rest VALUE ...]"), "{help}");
    }

    #[test]
    fn invalid_choices_and_unknown_options_exit_with_usage() {
        let parser = Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![Action::help()],
            choices: vec![Choice {
                name: "validate",
                help: "validate the model",
                parser: Parser {
                    prog: "planning validate".into(),
                    description: None,
                    actions: vec![Action::help()],
                    choices: vec![],
                },
            }],
        };
        let outcome = parser.parse_args(&["bogus".to_string()], &terminal());
        assert!(
            matches!(&outcome, Outcome::Exit { code: 2, stderr, .. }
                if stderr.contains("unrecognized arguments: bogus")),
            "{outcome:?}"
        );
        let choosy = Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![Action {
                options: vec!["--mode"],
                dest: "mode",
                nargs: Nargs::One,
                required: false,
                help: Some("a mode"),
                metavar: Some("MODE"),
                kind: Kind::Choice(&["fast", "slow"]),
            }],
            choices: vec![],
        };
        let outcome = choosy.parse_args(&["--mode".to_string(), "z".to_string()], &terminal());
        assert!(
            matches!(&outcome, Outcome::Exit { code: 2, stderr, .. }
                if stderr.contains("invalid choice: 'z'")),
            "{outcome:?}"
        );
        let outcome = parser.parse_args(&["--nope".to_string()], &terminal());
        assert!(
            matches!(&outcome, Outcome::Exit { code: 2, stderr, .. } if stderr.contains("--nope")),
            "{outcome:?}"
        );
    }

    #[test]
    fn equals_forms_and_missing_values_parse_like_argparse() {
        let parser = Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![Action {
                options: vec!["--name"],
                dest: "name",
                nargs: Nargs::One,
                required: false,
                help: Some("a name"),
                metavar: Some("NAME"),
                kind: Kind::Store,
            }],
            choices: vec![],
        };
        let outcome = parser.parse_args(&["--name=bob".to_string()], &terminal());
        assert!(
            matches!(&outcome, Outcome::Parsed(ns) if ns.str("name") == Some("bob")),
            "{outcome:?}"
        );
        let outcome = parser.parse_args(&["--name".to_string()], &terminal());
        assert!(
            matches!(&outcome, Outcome::Exit { code: 2, stderr, .. }
                if stderr.contains("expected one argument")),
            "{outcome:?}"
        );
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|a| a.to_string()).collect()
    }

    /// Two positionals, the first optional, with metavars long enough that the
    /// usage line wraps; argparse 3.15 with COLUMNS=80 printed the usage below.
    fn positionals_only() -> Parser {
        Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![
                Action {
                    options: vec![],
                    dest: "maybe",
                    nargs: Nargs::Optional,
                    required: false,
                    help: None,
                    metavar: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
                    kind: Kind::Store,
                },
                Action {
                    options: vec![],
                    dest: "one",
                    nargs: Nargs::One,
                    required: true,
                    help: None,
                    metavar: Some("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
                    kind: Kind::Store,
                },
            ],
            choices: vec![],
        }
    }

    #[test]
    fn a_long_usage_of_positionals_only_wraps_under_the_first() {
        assert_eq!(
            positionals_only().format_usage(&terminal(), false),
            "usage: planning [AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA]\n                \
             BBBBBBBBBBBBBBBBBBBBBBBBBBBBBB\n"
        );
    }

    #[test]
    fn an_optional_positional_gives_way_to_a_required_one() {
        // argparse: parse_args(["x"]) gives maybe=None, one='x'.
        let outcome = positionals_only().parse_args(&args(&["x"]), &terminal());
        assert!(
            matches!(&outcome, Outcome::Parsed(ns)
                if ns.values["maybe"] == Value::None && ns.str("one") == Some("x")),
            "{outcome:?}"
        );
        let outcome = positionals_only().parse_args(&args(&["x", "y"]), &terminal());
        assert!(
            matches!(&outcome, Outcome::Parsed(ns)
                if ns.str("maybe") == Some("x") && ns.str("one") == Some("y")),
            "{outcome:?}"
        );
    }

    fn quiet() -> Parser {
        Parser {
            prog: "planning".into(),
            description: None,
            actions: vec![
                Action::help(),
                Action {
                    options: vec!["--quiet"],
                    dest: "quiet",
                    nargs: Nargs::Optional,
                    required: false,
                    help: Some(" "),
                    metavar: Some("Q"),
                    kind: Kind::Store,
                },
            ],
            choices: vec![],
        }
    }

    #[test]
    fn blank_help_ends_the_invocation_line() {
        // argparse 3.15, add_argument("--quiet", nargs="?", help=" ", metavar="Q").
        assert_eq!(
            quiet().format_help(&terminal(), false),
            "usage: planning [-h] [--quiet [Q]]\n\noptions:\n  -h, --help   show this help \
             message and exit\n  --quiet [Q]  \n"
        );
    }

    #[test]
    fn an_optional_value_may_be_left_off() {
        let outcome = quiet().parse_args(&args(&["--quiet"]), &terminal());
        assert!(
            matches!(&outcome, Outcome::Parsed(ns) if ns.values["quiet"] == Value::None),
            "{outcome:?}"
        );
        let outcome = quiet().parse_args(&args(&["--quiet", "v"]), &terminal());
        assert!(
            matches!(&outcome, Outcome::Parsed(ns) if ns.str("quiet") == Some("v")),
            "{outcome:?}"
        );
    }

    #[test]
    fn decolor_keeps_escapes_that_are_not_colour_codes() {
        assert_eq!(
            decolor("\x1b[1mbold\x1b[0m \x1bxm \x1bplain"),
            "bold \x1bxm \x1bplain"
        );
    }

    #[test]
    fn finish_collapses_blank_lines_and_ends_with_one_newline() {
        assert_eq!(finish("a\n\n\n\nb\n\n\n"), "a\n\nb\n");
        assert_eq!(finish("a"), "a\n");
        assert_eq!(finish(""), "");
    }

    #[test]
    fn nargs_patterns_follow_get_nargs_pattern() {
        let action = |options: Vec<&'static str>, nargs| Action {
            options,
            dest: "x",
            nargs,
            required: false,
            help: None,
            metavar: None,
            kind: Kind::Store,
        };
        // _get_nargs_pattern, with '-*' and '-' removed for an optional.
        for (nargs, positional, optional) in [
            (Nargs::One, "-*A-*", "A"),
            (Nargs::Optional, "-*A?-*", "A?"),
            (Nargs::Parser, "-*A[-AO]*", "A[AO]*"),
        ] {
            assert_eq!(Parser::nargs_pattern(&action(vec![], nargs)), positional);
            assert_eq!(Parser::nargs_pattern(&action(vec!["--x"], nargs)), optional);
        }
        assert_eq!(Parser::nargs_pattern(&Action::help()), "");
        assert_eq!(Formatter::format_args(&Action::help(), "X"), "");
    }

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn popular_elements_extend_a_match_as_difflib_does() {
        // b is 253 long, so 'e' (250 of them) is popular and left out of b2j.
        // CPython's difflib finds Match(a=1, b=0, size=253) and
        // Match(a=0, b=0, size=253), each with ratio 2 * 253 / 507.
        let b = chars(&format!("{}xyz", "e".repeat(250)));
        let a = chars(&format!("q{}xyz", "e".repeat(250)));
        let matcher = SequenceMatcher::new(&a, &b);
        assert_eq!(matcher.find_longest_match(0, 254, 0, 253), (1, 0, 253));
        assert_eq!(matcher.ratio(), 2.0 * 253.0 / 507.0);
        let b = chars(&format!("xyz{}", "e".repeat(250)));
        let a = chars(&format!("xyz{}q", "e".repeat(250)));
        let matcher = SequenceMatcher::new(&a, &b);
        assert_eq!(matcher.find_longest_match(0, 254, 0, 253), (0, 0, 253));
        assert_eq!(matcher.ratio(), 2.0 * 253.0 / 507.0);
    }

    #[test]
    fn a_popular_run_between_unpopular_elements_still_matches_in_full() {
        // CPython 3.15's difflib gives these matched totals and ratios. Its
        // find_longest_match can return a longer first block here, (0, 0, 252)
        // where this gives (0, 0, 251), but get_matching_blocks recovers the
        // rest, so the totals close_match ranks by agree.
        let e = "e".repeat(250);
        let e120 = "e".repeat(120);
        for (a, b, matched, ratio) in [
            (format!("x{e}y"), format!("x{e}y"), 252, 1.0),
            (format!("x{e}z"), format!("x{e}y"), 251, 0.996031746031746),
            (format!("q{e}y"), format!("x{e}y"), 251, 0.996031746031746),
            (
                format!("x{e120}y{e120}w"),
                format!("x{e120}y{e120}z"),
                242,
                0.9958847736625515,
            ),
            (
                format!("ab{e}cx"),
                format!("ab{e}cd"),
                253,
                0.9960629921259843,
            ),
        ] {
            let (a, b) = (chars(&a), chars(&b));
            let matcher = SequenceMatcher::new(&a, &b);
            assert_eq!(matcher.matches(), matched);
            assert_eq!(matcher.ratio(), ratio);
        }
    }

    #[test]
    fn two_empty_sequences_match_completely() {
        // SequenceMatcher(None, "", "").ratio() is 1.0.
        assert_eq!(SequenceMatcher::new(&[], &[]).ratio(), 1.0);
    }
}

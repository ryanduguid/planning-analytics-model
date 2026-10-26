use std::io::{BufWriter, LineWriter, Write};

use pacioliscube::TextStream;
use pacioliscube::cli::{self, Streams};
use pacioliscube::py;
use pacioliscube::terminal::Terminal;

/// Deep hierarchies and rule chains recurse once per level; the stack is
/// sized so that depths far beyond any planning model still evaluate.
const STACK_BYTES: usize = 1 << 30;

/// Run the command, then flush stdout as interpreter shutdown did: output
/// still pending there, including what an earlier failed write left behind,
/// fails the way CPython reports it, with exit status 120.
fn run_with(out: impl Write, argv: &[String], terminal: &Terminal) -> i32 {
    let mut out = TextStream::new(out);
    let mut err = TextStream::new(std::io::stderr());
    let mut code = cli::main(
        argv,
        terminal,
        &mut Streams {
            out: &mut out,
            err: &mut err,
        },
    );
    if let Err(error) = out.flush() {
        let (name, text) = py::stream_oserror(&error);
        let _ = writeln!(
            err,
            "Exception ignored while flushing sys.stdout:\n{name}: {text}"
        );
        code = cli::EXIT_FLUSH_FAILED;
    }
    let _ = err.flush();
    code
}

fn run() -> i32 {
    // CPython reads PYTHONINTMAXSTRDIGITS before anything else and stops on a bad value.
    if py::int_max_str_digits().is_none() {
        let mut err = TextStream::new(std::io::stderr());
        let _ = err.write_all(py::INT_MAX_STR_DIGITS_FATAL.as_bytes());
        return 1;
    }
    let mut argv = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(arg) => argv.push(arg),
            Err(_) => {
                let mut err = TextStream::new(std::io::stderr());
                let _ = writeln!(
                    err,
                    "pacioliscube: unexpected error: UnicodeError: a command line argument is not \
                     UTF-8 text"
                );
                return 1;
            }
        }
    }
    // The MCP server sits outside the parser, whose help text reproduces the
    // Python engine's byte for byte.
    if argv.first().is_some_and(|command| command == "mcp") {
        return pacioliscube::mcp::main(&argv[1..]);
    }
    let terminal = Terminal::detect();
    // Python flushes stdout by line on a terminal and buffers it elsewhere.
    // PYTHONUNBUFFERED writes each piece straight to the file, so a failed
    // write ends the command and leaves nothing pending for shutdown; stderr
    // is written as each line is printed.
    if terminal.stdout_tty {
        run_with(LineWriter::new(std::io::stdout()), &argv, &terminal)
    } else if let Some(raw) = cli::python_unbuffered()
        .then(raw_stdout)
        .and_then(Result::ok)
    {
        run_with(raw, &argv, &terminal)
    } else {
        run_with(
            BufWriter::with_capacity(128 * 1024, std::io::stdout()),
            &argv,
            &terminal,
        )
    }
}

/// stdout with no buffer at all: a duplicate of its handle, written directly.
#[cfg(unix)]
fn raw_stdout() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsFd;
    Ok(std::io::stdout().as_fd().try_clone_to_owned()?.into())
}

/// stdout with no buffer at all: a duplicate of its handle, written directly.
#[cfg(windows)]
fn raw_stdout() -> std::io::Result<std::fs::File> {
    use std::os::windows::io::AsHandle;
    Ok(std::io::stdout().as_handle().try_clone_to_owned()?.into())
}

fn main() {
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(run)
        .expect("the engine thread starts");
    let code = worker.join().unwrap_or(1);
    std::process::exit(code);
}

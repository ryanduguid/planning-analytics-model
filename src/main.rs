use std::io::{BufWriter, LineWriter, Write};

use pacioliscube::TextStream;
use pacioliscube::cli::{self, Streams};
use pacioliscube::py;
use pacioliscube::terminal::Terminal;

/// Deep hierarchies and rule chains recurse once per level; the stack is
/// sized so that depths far beyond any planning model still evaluate.
const STACK_BYTES: usize = 1 << 30;

/// Python's exit status when flushing a standard stream fails at shutdown.
const EXIT_FLUSH_FAILED: i32 = 120;

/// stdout as the command wrote it, remembering whether a write failed: that
/// failure has already ended the command, as Python's OSError backstop did.
struct Watched<W: Write> {
    inner: W,
    failed: bool,
}

impl<W: Write> Write for Watched<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf);
        self.failed |= written.is_err();
        written
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Run the command, then flush stdout as interpreter shutdown did: a failure
/// there is reported the way CPython reports it, with exit status 120.
fn run_with(out: impl Write, argv: &[String], terminal: &Terminal) -> i32 {
    let mut out = Watched {
        inner: TextStream::new(out),
        failed: false,
    };
    let mut err = TextStream::new(std::io::stderr());
    let mut code = cli::main(
        argv,
        terminal,
        &mut Streams {
            out: &mut out,
            err: &mut err,
        },
    );
    if let Err(error) = out.flush()
        && !out.failed
    {
        let (name, text) = py::stream_oserror(&error);
        let _ = writeln!(
            err,
            "Exception ignored while flushing sys.stdout:\n{name}: {text}"
        );
        code = EXIT_FLUSH_FAILED;
    }
    let _ = err.flush();
    code
}

fn run() -> i32 {
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
    let terminal = Terminal::detect();
    // Python buffers stdout when it is not a terminal and flushes it by line
    // when it is; stderr is written as each line is printed.
    if terminal.stdout_tty {
        run_with(LineWriter::new(std::io::stdout()), &argv, &terminal)
    } else {
        run_with(
            BufWriter::with_capacity(128 * 1024, std::io::stdout()),
            &argv,
            &terminal,
        )
    }
}

fn main() {
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(run)
        .expect("the engine thread starts");
    let code = worker.join().unwrap_or(1);
    std::process::exit(code);
}

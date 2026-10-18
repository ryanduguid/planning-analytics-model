use std::io::{BufWriter, LineWriter, Write};

use pacioliscube::TextStream;
use pacioliscube::cli::{self, Streams};
use pacioliscube::terminal::Terminal;

/// Deep hierarchies and rule chains recurse once per level; the stack is
/// sized so that depths far beyond any planning model still evaluate.
const STACK_BYTES: usize = 1 << 30;

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
    let mut err = TextStream::new(std::io::stderr());
    // Python buffers stdout when it is not a terminal and flushes it by line
    // when it is; stderr is written as each line is printed.
    let code = if terminal.stdout_tty {
        let mut out = TextStream::new(LineWriter::new(std::io::stdout()));
        let code = cli::main(
            &argv,
            &terminal,
            &mut Streams {
                out: &mut out,
                err: &mut err,
            },
        );
        let _ = out.flush();
        code
    } else {
        let mut out = TextStream::new(BufWriter::with_capacity(128 * 1024, std::io::stdout()));
        let code = cli::main(
            &argv,
            &terminal,
            &mut Streams {
                out: &mut out,
                err: &mut err,
            },
        );
        let _ = out.flush();
        code
    };
    let _ = err.flush();
    code
}

fn main() {
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(run)
        .expect("the engine thread starts");
    let code = worker.join().unwrap_or(1);
    std::process::exit(code);
}

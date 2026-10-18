//! An open IBM Planning Analytics budgeting model and an offline engine for it.
//!
//! The engine is driven through the `pacioliscube` command line; the modules
//! here are its implementation and carry no stability promise of their own.

pub mod argparse;
pub mod cli;
pub mod csvread;
pub mod data;
pub mod decimal;
pub mod demo;
pub mod error;
pub mod evaluate;
pub mod fuzz;
pub mod model;
pub mod py;
pub mod pyjson;
pub mod pypath;
mod pyunicode;
pub mod report;
pub mod rules;
pub mod terminal;
pub mod validate;

/// A writer that ends lines as Python's text-mode streams did on this
/// platform: `\r\n` on Windows, `\n` elsewhere.
pub struct TextStream<W: std::io::Write> {
    inner: W,
}

impl<W: std::io::Write> TextStream<W> {
    pub fn new(inner: W) -> TextStream<W> {
        TextStream { inner }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: std::io::Write> std::io::Write for TextStream<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if cfg!(windows) {
            let mut start = 0;
            for (i, b) in buf.iter().enumerate() {
                if *b == b'\n' {
                    self.inner.write_all(&buf[start..i])?;
                    self.inner.write_all(b"\r\n")?;
                    start = i + 1;
                }
            }
            self.inner.write_all(&buf[start..])?;
        } else {
            self.inner.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

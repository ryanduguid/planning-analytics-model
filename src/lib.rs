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
    /// One write per call, translated first, as `TextIOWrapper.write` hands
    /// its buffer one encoded chunk: what stays pending after a failed write
    /// then matches Python's.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if cfg!(windows) && buf.contains(&b'\n') {
            let mut translated = Vec::with_capacity(buf.len() + buf.len() / 16);
            for b in buf {
                if *b == b'\n' {
                    translated.push(b'\r');
                }
                translated.push(*b);
            }
            self.inner.write_all(&translated)?;
        } else {
            self.inner.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

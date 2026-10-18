The cargo-fuzz target feeds fabricated bytes to the TM1 rules parser. It
checks that every input either parses or is refused as a rules syntax error,
that parsing the same text twice gives the same rule set, and that each rule's
qualifier is one the grammar allows. Unexpected failures and failed invariants
fail the run. The seed corpus holds the model's own rules files and a few edge
cases.

libFuzzer instrumentation needs a nightly compiler, so the toolchain date is
pinned. On Linux, run:

```sh
rustup toolchain install nightly-2026-10-15 --profile minimal
cargo install cargo-fuzz --locked --version 0.13.2
cargo +nightly-2026-10-15 fuzz run rules_parser fuzz/corpus -- -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

CI runs the target after main pushes, weekly and on demand, with read-only
repository access. The test suite replays the seed corpus on every supported
platform (`tests/repository.rs`). A successful run is bounded evidence, not
proof that every input is handled.

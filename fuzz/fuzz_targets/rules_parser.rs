//! Fabricated bytes to the TM1 rules parser.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Err(problem) = pacioliscube::fuzz::check_input(data) {
        panic!("{problem}");
    }
});

#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    if let Ok(clean) = appspec::icon::sanitize(bytes) {
        let again = appspec::icon::sanitize(&clean);
        // Re-encoded PNGs can exceed the input cap; small ones are stable.
        if clean.len() <= appspec::icon::MAX_BYTES {
            assert_eq!(again.unwrap(), clean);
        }
    }
});

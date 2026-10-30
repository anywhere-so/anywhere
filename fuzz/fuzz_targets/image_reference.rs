//! An image app's `image`: `appspec::ImageRef::parse`, which the CLI, the
//! control plane and the helper that pulls it run on what `app.toml` says. A
//! reference that parses writes out in full and reads back the same, and asks
//! the registry for a tag or a digest.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let image = match appspec::ImageRef::parse(text) {
        Ok(image) => image,
        Err(error) => {
            assert!(!error.to_string().is_empty());
            return;
        }
    };
    assert!(!image.registry.is_empty());
    assert!(!image.repository.is_empty());
    assert!(!image.api_host().is_empty());
    assert!(!image.reference().is_empty());
    assert_eq!(appspec::ImageRef::parse(&image.to_string()).unwrap(), image);
});

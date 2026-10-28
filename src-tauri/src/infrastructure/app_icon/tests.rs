//! Every picture this build can put on, read the way it is put on.

use super::decoded;
use lyonoyl_domain::preferences::AppIcon;

/// The pictures for one place, one for each choice, as files beside the code.
fn pictures(folder: &str) -> Vec<(String, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("icons/choice")
        .join(folder);
    AppIcon::ALL
        .into_iter()
        .map(|icon| format!("{}.png", icon.as_str()))
        .map(|name| {
            let bytes = std::fs::read(root.join(&name))
                .unwrap_or_else(|why| panic!("{folder}/{name}: {why}"));
            (name, bytes)
        })
        .collect()
}

#[test]
fn every_picture_a_window_on_windows_can_wear_opens_as_its_pixels() {
    for (name, bytes) in pictures("windows") {
        let image =
            decoded(&bytes).unwrap_or_else(|| panic!("windows/{name} is not eight-bit RGBA"));
        assert_eq!(
            (image.width(), image.height()),
            (256, 256),
            "windows/{name}"
        );
        assert_eq!(image.rgba().len(), 256 * 256 * 4, "windows/{name}");
    }
}

#[test]
fn every_picture_the_dock_can_wear_is_there_at_the_size_it_is_shown() {
    for (name, bytes) in pictures("macos") {
        let image = decoded(&bytes).unwrap_or_else(|| panic!("macos/{name} is not eight-bit RGBA"));
        assert_eq!((image.width(), image.height()), (512, 512), "macos/{name}");
    }
}

#[test]
fn something_that_is_not_a_picture_is_worn_as_nothing() {
    assert!(decoded(b"not a picture").is_none());
    assert!(decoded(&[]).is_none());
}

//! Paste collapsing and attachment tests. See `SPEC-tui-experience` sections 5 and 6.
//!
//! The assertion that matters most is `large_paste_still_reaches_the_model`. A
//! collapsed paste that loses its text is a data-loss defect, not a display choice.
//! Anything that writes is isolated with `tempfile`.

use rho_tui::{
    AttachOutcome, BurstKey, COMPOSER_MAX_TEXT_ROWS, Composer, ImageChip, RoutedInput,
    attach_image, paste_chip_label, route_burst,
};

// --- Paste collapsing.

#[test]
fn paste_over_threshold_collapses_to_chip() {
    let mut composer = Composer::new();
    let chip = composer
        .paste(&"x".repeat(1200))
        .expect("a 1200-char paste collapses to a chip");
    assert_eq!(paste_chip_label(&chip), "[paste 1200 chars]");
}

#[test]
fn paste_repeat_suffix() {
    let mut composer = Composer::new();
    let text = "y".repeat(1200);
    composer.paste(&text);
    let second = composer
        .paste(&text)
        .expect("a second same-size paste collapses too");
    assert_eq!(paste_chip_label(&second), "[paste 1200 chars #2]");
}

#[test]
fn large_paste_still_reaches_the_model() {
    let mut composer = Composer::new();
    let text = "z".repeat(1500);
    composer.paste(&text);
    assert!(
        composer.model_text().contains(&text),
        "the full pasted text must still reach the model on send"
    );
}

#[test]
fn paste_chip_deletes_as_one_unit() {
    let mut composer = Composer::new();
    let text = "w".repeat(1200);
    composer.paste(&text);
    assert_eq!(composer.chip_count(), 1, "the paste is held as one chip");
    assert!(composer.backspace(), "one backspace removes the whole chip");
    assert_eq!(composer.chip_count(), 0, "the chip is gone");
    assert!(
        !composer.model_text().contains(&text),
        "the held text goes with the chip"
    );
}

#[test]
fn paste_burst_does_not_open_the_help() {
    // A burst of `?` keys arrives faster than a human types, so it collapses to one
    // paste and flushes through the paste path.
    let burst: Vec<BurstKey> = (0..8)
        .map(|_| BurstKey {
            ch: '?',
            gap_millis: 1,
        })
        .collect();
    let routed = route_burst(&burst);
    assert!(
        !routed
            .iter()
            .any(|input| matches!(input, RoutedInput::Key('?'))),
        "a pasted ? must never become a help shortcut, so the help stays shut: {routed:?}"
    );
    assert!(
        routed
            .iter()
            .any(|input| matches!(input, RoutedInput::Paste(_))),
        "the burst must flush through the paste path: {routed:?}"
    );
}

#[test]
fn composer_height_is_bounded() {
    let mut composer = Composer::new();
    composer.insert(&"line\n".repeat(20));
    assert!(
        composer.height_rows() <= COMPOSER_MAX_TEXT_ROWS + 2,
        "a tall draft caps at ten rows with borders and scrolls inside, got {}",
        composer.height_rows()
    );
}

// --- Attachments.

#[test]
fn image_chip_label() {
    // 1.2 MB, as bytes. 1.2 * 1024 * 1024, rounded.
    let chip = ImageChip {
        index: 1,
        bytes: 1_258_291,
    };
    // The test name matches the spec, so the function is called fully qualified.
    assert_eq!(rho_tui::image_chip_label(&chip), "[image #1 1.2MB]");
}

#[test]
fn oversize_attachment_is_refused() {
    let root = tempfile::tempdir().expect("a temp session root");
    let path = root.path().join("big.png");
    std::fs::write(&path, b"stand-in bytes").expect("write the stand-in file");
    let outcome = attach_image(root.path(), &path, 12 * 1024 * 1024);
    assert!(
        matches!(outcome, AttachOutcome::Refused(_)),
        "a 12MB image is refused and never sent: {outcome:?}"
    );
}

#[test]
fn oversize_attachment_states_the_limit() {
    let root = tempfile::tempdir().expect("a temp session root");
    let path = root.path().join("big.png");
    std::fs::write(&path, b"stand-in bytes").expect("write the stand-in file");
    match attach_image(root.path(), &path, 12 * 1024 * 1024) {
        AttachOutcome::Refused(message) => {
            assert!(message.contains("12MB"), "names the size: {message}");
            assert!(
                message.contains("the limit is 5MB"),
                "names the 5 MB limit: {message}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn attachment_path_outside_root_is_refused() {
    let root = tempfile::tempdir().expect("a temp session root");
    let outside = tempfile::tempdir().expect("a temp dir outside the root");
    let path = outside.path().join("secret.png");
    std::fs::write(&path, b"secret").expect("write the outside file");
    let outcome = attach_image(root.path(), &path, 100);
    assert!(
        matches!(outcome, AttachOutcome::Refused(_)),
        "a path outside the session root is refused: {outcome:?}"
    );
}

use super::*;

#[test]
fn arrows_and_text_decode_to_keys() {
    assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
    assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
    assert_eq!(decode(b"\x1b[C"), vec![Key::Right]);
    assert_eq!(decode(b"\x1bOD"), vec![Key::Left]);
    assert_eq!(decode(b"\x1b[5~"), vec![Key::PageUp]);
    assert_eq!(decode(b"\x1b"), vec![Key::Cancel]);
    assert_eq!(decode(b"\x1b[H"), vec![]);
    assert_eq!(decode(b"\r"), vec![Key::Enter]);
    assert_eq!(decode(b"\x03"), vec![Key::Interrupt]);
    assert_eq!(decode(b"lu"), vec![Key::Char('l'), Key::Char('u')]);
}

#[test]
fn the_page_fits_the_screen() {
    assert_eq!(page_for(Some(24)), 15);
    assert_eq!(page_for(Some(12)), 3);
    assert_eq!(page_for(Some(5)), 1);
    assert_eq!(page_for(None), 10);
}

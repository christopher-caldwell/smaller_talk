#[test]
fn transcript_lines_do_not_reserve_a_kibibyte_of_unused_inline_storage() {
    // The history workload shapes tens of thousands of lines. Most have very
    // few decorations; complex lines may grow their decoration storage on the
    // heap. Bound the cost paid by every line, independently of its content.
    assert!(std::mem::size_of::<gpui::ShapedLine>() < 1024);
    assert!(std::mem::size_of::<gpui::WrappedLine>() < 1024);
}

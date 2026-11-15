//! The reference model's input (`LLEVANT_BIG_MODEL`, `--ignored`): how long
//! a card edit takes before compiling (reading the cards; one edit in place,
//! and one that moves the lines after it; writing the input).

use std::time::Instant;

use llevant_scene::source::{CardKind, Source};

#[test]
#[ignore]
fn big_model_card_edit() {
    let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let t = Instant::now();
    let text = std::fs::read_to_string(path).unwrap();
    let read = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let src = Source::from_str(&text);
    let cards = t.elapsed().as_secs_f64();
    let id = src
        .tree()
        .surfaces()
        .filter_map(|s| s.id())
        .nth(1000)
        .unwrap();
    let card = src.surface(id).unwrap();
    let line = card.line;
    let card = card.text.trim_end().to_string();
    let t = Instant::now();
    let e = src
        .edit(CardKind::Surface, id, &format!("{card} $ edited"))
        .unwrap();
    let edit = t.elapsed().as_secs_f64();
    assert_eq!(e.old.trim_end(), card);
    assert_eq!(src.surface(id).unwrap().line, line);
    let t = Instant::now();
    let e2 = src
        .edit(
            CardKind::Surface,
            id,
            &format!("c a comment\n{card} $ edited"),
        )
        .unwrap();
    let moved = t.elapsed().as_secs_f64();
    // The comment is the card's first line now: its first token one down.
    assert_eq!(src.surface(id).unwrap().line, line + 1);
    let t = Instant::now();
    let mut out = Vec::with_capacity(text.len() + 64);
    src.write_source(&mut out).unwrap();
    let write = t.elapsed().as_secs_f64();
    assert_eq!(out.len(), text.len() + "c a comment\n $ edited".len());
    src.restore(&e2).unwrap();
    src.restore(&e).unwrap();
    assert!(src.to_source() == text, "edits taken back");
    eprintln!(
        "{} MB: read {read:.2} s, cards {cards:.2} s, one card edited {edit:.4} s, \
         one moving the lines {moved:.3} s, written {write:.2} s",
        text.len() / 1_000_000
    );
}

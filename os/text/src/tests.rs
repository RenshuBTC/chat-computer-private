//! Lines set as HarfBuzz sets them (reference.py), kerning and all, and measured, wrapped and drawn the same way.

use super::*;

// Where HarfBuzz sets each glyph along the line (font units), at the regular weight or the bold, and the line's width.
const SET: [(&str, bool, &[i32], i32); 12] = [
    ("AVATAR", false, &[0, 586, 1139, 1715, 2223, 2849], 3463),
    ("AVATAR", true, &[0, 630, 1226, 1838, 2379, 3057], 3707),
    ("Today, Tokyo.", false, &[0, 478, 1029, 1593, 2111, 2530, 2740, 3000, 3478, 4026, 4523, 4964, 5502], 5712),
    ("Today, Tokyo.", true, &[0, 527, 1104, 1700, 2234, 2701, 2972, 3264, 3791, 4367, 4931, 5434, 6000], 6272),
    ("WAVY lines: P. F, y. r.", false, &[0, 826, 1412, 2034, 2579, 2859, 3128, 3380, 3930, 4468, 4939, 5149, 5429, 5961, 6171, 6451, 6943, 7153, 7423, 7842, 8052, 8332, 8611], 8821),
    ("WAVY lines: P. F, y. r.", true, &[0, 848, 1478, 2151, 2771, 3083, 3380, 3680, 4251, 4814, 5322, 5594, 5906, 6475, 6747, 7059, 7573, 7844, 8138, 8605, 8877, 9189, 9495], 9767),
    ("“Quoted” — LT AV To", false, &[0, 399, 1123, 1663, 2198, 2512, 3050, 3614, 4013, 4293, 5117, 5397, 5868, 6406, 6676, 7262, 7835, 8095, 8573], 9121),
    ("“Quoted” — LT AV To", true, &[0, 479, 1235, 1802, 2376, 2738, 3301, 3897, 4376, 4688, 5520, 5832, 6298, 6885, 7187, 7817, 8441, 8733, 9260], 9836),
    ("Hello, world", false, &[0, 694, 1232, 1489, 1747, 2285, 2495, 2765, 3441, 3989, 4348, 4606], 5170),
    ("Hello, world", true, &[0, 701, 1264, 1572, 1876, 2442, 2713, 3007, 3729, 4305, 4691, 4995], 5591),
    (" T A Y ", false, &[0, 260, 798, 1068, 1684, 1934, 2479], 2759),
    (" T A Y ", true, &[0, 292, 879, 1181, 1849, 2131, 2751], 3063),
];

/// A size at which a font unit is a 256th of a pixel: widths come out in font units.
const UNIT: i32 = 1000;

fn starts(ty: &Type, text: &str, face: Face) -> Vec<i32> {
    ty.line(text, face).map(|(x, _)| x).collect()
}

#[test]
fn sets_letters_where_harfbuzz_does() {
    let ty = Type::atkinson();
    for (text, bold, want, width) in SET {
        let face = if bold { Face::Bold } else { Face::Regular };
        assert_eq!(starts(&ty, text, face), want, "{text:?}, bold {bold}");
        assert_eq!(ty.width_in(text, face, UNIT), width, "{text:?}, bold {bold}");
        // Slanted, the same glyphs in the same places.
        let slanted = if bold { Face::BoldItalic } else { Face::Italic };
        assert_eq!(starts(&ty, text, slanted), want, "{text:?} slanted");
    }
    // Kerned closer than the letters' own widths.
    let plain: i32 = "AVATAR".chars().map(|c| ty.set(c, Face::Regular).unwrap().advance).sum();
    assert!(ty.width("AVATAR", UNIT) < plain - 100);
}

#[test]
fn kerns_only_within_one_font_and_face() {
    let ty = Type::atkinson();
    let (r, b) = (Face::Regular, Face::Bold);
    assert!(ty.kern_between(('A', r), ('V', r), UNIT) < 0 && ty.kern_between(('A', b), ('V', b), UNIT) < 0);
    // Not across a change of face, nor in code (its font doesn't kern), nor with a fallback font's character.
    assert_eq!(ty.kern_between(('A', r), ('V', b), UNIT), 0);
    assert_eq!(ty.kern_between(('A', Face::Mono), ('V', Face::Mono), UNIT), 0);
    assert_eq!(ty.kern_between(('T', r), ('→', r), UNIT), 0);
    // Something that shows nothing doesn't come between letters that kern; a line break does.
    assert_eq!(starts(&ty, "A\u{200B}V", r), starts(&ty, "AV", r));
    assert_eq!(ty.width("A\u{FE0F}V", UNIT), ty.width("AV", UNIT));
}

#[test]
fn wraps_kerned_text_as_wide_as_it_is_drawn() {
    let ty = Type::atkinson();
    // (At this size widths are font units, exactly: no rounding between measuring and wrapping.)
    let size = UNIT;
    // Kerned, it fits in its own width (it wouldn't, unkerned); a unit less and it breaks inside the word.
    let word = "AVAVAVAV";
    let w = ty.width(word, size);
    assert_eq!(ty.breaks(word, size, w), vec![0..8]);
    assert_eq!(ty.breaks(word, size, w - 1).len(), 2);
    let styled = |t: &str| t.chars().map(|c| (c, Face::Regular)).collect::<Vec<_>>();
    assert_eq!(ty.breaks_styled(&styled(word), size, w), vec![0..8]);
    // Broken at a space, the next line starts with the word after it, not kerned with the space (" T" is closer):
    // "ToT" is a unit too wide for the last line, however close the space would have made it.
    let text = "To To ToT";
    let max = ty.width("ToT", size) - 1;
    assert!(ty.kern_between((' ', Face::Regular), ('T', Face::Regular), size) < 0);
    assert_eq!(ty.breaks(text, size, max), vec![0..2, 3..5, 6..8, 8..9]);
    assert_eq!(ty.breaks_styled(&styled(text), size, max), vec![0..2, 3..5, 6..8, 8..9]);
    assert_eq!(ty.breaks(text, size, max + 1), vec![0..2, 3..5, 6..9]);
    // While the line goes on, a space's own kerning counts ("T " and " A" are closer).
    let spaced = ty.width("T A", size);
    assert!(spaced < ty.width("T", size) + ty.width(" ", size) + ty.width("A", size));
    assert_eq!(ty.breaks("T A", size, spaced), vec![0..3]);
}

#[test]
fn the_caret_goes_where_the_letter_starts() {
    let ty = Type::atkinson();
    let text = "AVATAR";
    let want = starts(&ty, text, Face::Regular);
    for (i, x) in want.iter().enumerate() {
        assert_eq!(ty.x_at(text, i, UNIT), *x, "before letter {i}");
    }
    assert_eq!(ty.x_at(text, text.len(), UNIT), ty.width(text, UNIT));
    assert_eq!(ty.x_at(text, 99, UNIT), ty.width(text, UNIT));
    assert_eq!(ty.x_at("", 0, UNIT), 0);
}

#[test]
fn draws_kerned_lines_as_wide_as_measured() {
    let ty = Type::atkinson();
    let style = Style { size: 24 * ONE, line_height: 36 * ONE };
    for face in [Face::Regular, Face::Bold] {
        let ink = ty.ink_in("AVATAR", face, style, 0);
        assert_eq!(ink.width, ((ty.width_in("AVATAR", face, style.size) + ONE - 1) / ONE) as usize + 2 * PAD);
        // Ink in the last column a letter can reach, none past the pad.
        let column = |x: usize| (0..ink.height).map(|y| ink.coverage[y * ink.width + x] as u32).sum::<u32>();
        assert!(column(ink.width - PAD - 2) > 0 && column(ink.width - 1) == 0, "{face:?}");
    }
}

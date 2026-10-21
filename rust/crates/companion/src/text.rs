//! The Gen 3 English games' text encoding (charmap.txt in the pret decomps): the part names use.

fn char_of(c: u8) -> &'static str {
    const UPPER: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    const LOWER: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match c {
        0xBB..=0xD4 => UPPER[(c - 0xBB) as usize],
        0xD5..=0xEE => LOWER[(c - 0xD5) as usize],
        0xA1..=0xAA => DIGITS[(c - 0xA1) as usize],
        0x00 => " ",
        0x1B => "é",
        0xAB => "!",
        0xAC => "?",
        0xAD => ".",
        0xAE => "-",
        0xB0 => "...",
        0xB1 | 0xB2 => "\"",
        0xB3 | 0xB4 => "'",
        0xB5 => "♂",
        0xB6 => "♀",
        0xB8 => ",",
        0xBA => "/",
        0xF0 => ":",
        _ => "?",
    }
}

/// Arguments after each 0xFC control code (the app's FC_ARGS).
const FC_ARGS: [u8; 25] = [
    0, 1, 1, 1, 3, 1, 1, 0, 1, 0, 0, 2, 1, 1, 1, 0, 2, 1, 1, 1, 1, 0, 0, 0, 0,
];

/// Decodes `src` up to its 0xFF terminator: line breaks become one space, control codes are skipped, and
/// the result is trimmed.
pub fn decode(src: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < src.len() && src[i] != 0xFF {
        let c = src[i];
        let s = match c {
            0xFE | 0xFA | 0xFB => " ",
            0xFC => {
                let code = src.get(i + 1).copied().unwrap_or(0) as usize;
                i += 2 + FC_ARGS.get(code).copied().unwrap_or(0) as usize;
                continue;
            }
            _ => char_of(c),
        };
        if !(s == " " && out.ends_with(' ')) {
            out.push_str(s);
        }
        i += 1;
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_name() {
        // "RED" then the terminator and junk after it.
        assert_eq!(decode(&[0xCC, 0xBF, 0xBE, 0xFF, 0xBB]), "RED");
        assert_eq!(
            decode(&[0xC8, 0xC3, 0xBE, 0xC9, 0xCC, 0xBB, 0xC8, 0xB6, 0xFF]),
            "NIDORAN♀"
        );
    }

    #[test]
    fn breaks_become_one_space() {
        assert_eq!(decode(&[0xBB, 0xFE, 0xFE, 0xBC, 0xFF]), "A B");
    }
}

//! Client-side respelling of dictated text: names the transcriber spells by sound ("Risha") are fixed to how they
//! are written ("Wrisha"). Whole words only, matched without case, and the case of the heard word is kept
//! (Risha -> Wrisha, risha -> wrisha, RISHA -> WRISHA). Add names to RESPELL.

/// (heard, written) pairs; `heard` is matched case-insensitively as a whole word.
const RESPELL: &[(&str, &str)] = &[("risha", "wrisha")];

/// respells every listed word in `text`; text with no listed word comes back unchanged
pub fn respell(text: &str) -> String {
    let mut out = text.to_string();
    for (heard, written) in RESPELL {
        out = replace_word(&out, heard, written);
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn replace_word(text: &str, heard: &str, written: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    let mut prev: Option<char> = None;
    while !rest.is_empty() {
        let hit = rest.len() >= heard.len()
            && rest.is_char_boundary(heard.len())
            && rest[..heard.len()].eq_ignore_ascii_case(heard)
            && !prev.is_some_and(is_word)
            && !rest[heard.len()..].chars().next().is_some_and(is_word);
        if hit {
            let found = &rest[..heard.len()];
            out.push_str(&match_case(found, written));
            prev = found.chars().last();
            rest = &rest[heard.len()..];
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            prev = Some(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// `written` in the case of `found`: all caps, capitalized or lower
fn match_case(found: &str, written: &str) -> String {
    let letters: Vec<char> = found.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        written.to_uppercase()
    } else if found.chars().next().is_some_and(|c| c.is_uppercase()) {
        let mut cs = written.chars();
        cs.next().map(|f| f.to_uppercase().chain(cs).collect()).unwrap_or_default()
    } else {
        written.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::respell;

    #[test]
    fn respells_the_name() {
        assert_eq!(respell("Risha Ghosh"), "Wrisha Ghosh");
        assert_eq!(respell("hi, I'm Risha."), "hi, I'm Wrisha.");
        assert_eq!(respell("risha said"), "wrisha said");
        assert_eq!(respell("RISHA"), "WRISHA");
        assert_eq!(respell("Risha's mic, risha"), "Wrisha's mic, wrisha");
    }

    #[test]
    fn leaves_other_words() {
        assert_eq!(respell("Wrisha Ghosh"), "Wrisha Ghosh");
        assert_eq!(respell("Rishab and Krisha, rishaa"), "Rishab and Krisha, rishaa");
        assert_eq!(respell(""), "");
        assert_eq!(respell("আমি Risha বলছি"), "আমি Wrisha বলছি");
        assert_eq!(respell("no names here"), "no names here");
    }
}

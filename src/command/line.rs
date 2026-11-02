//! The text being typed into the command line.

/// Most characters a line holds, so the prompt stays on screen.
pub const MAX_LEN: usize = 48;

/// A line being typed. Only lowercase letters, digits, spaces and minus signs get in, which
/// is everything a command is made of, so the text is always drawable and never needs escaping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    text: String,
}

impl Line {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Adds `c` at the end, as lowercase. Anything else, or a full line, is ignored.
    pub fn push(&mut self, c: char) {
        let c = c.to_ascii_lowercase();
        let wanted = c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == '-';
        if wanted && self.text.len() < MAX_LEN {
            self.text.push(c);
        }
    }

    /// Removes the last character.
    pub fn backspace(&mut self) {
        self.text.pop();
    }

    /// Replaces the text, as when a table row fills in the start of a command.
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        text.chars().for_each(|c| self.push(c));
    }

    /// Hands over the text and leaves the line empty.
    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.text)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn whatever_is_typed_the_line_stays_short_and_drawable(chars in prop::collection::vec(any::<char>(), 0..200)) {
            let mut line = Line::default();
            for c in chars {
                line.push(c);
            }
            prop_assert!(line.as_str().len() <= MAX_LEN);
            prop_assert!(line.as_str().chars().all(|c| crate::ui::font::glyph(c).is_some()));
        }
    }

    #[test]
    fn typing_backspace_and_taking_work_like_a_prompt() {
        let mut line = Line::default();
        "Give 5!".chars().for_each(|c| line.push(c));
        assert_eq!(line.as_str(), "give 5");
        line.backspace();
        assert_eq!(line.as_str(), "give ");
        assert_eq!(line.take(), "give ");
        assert_eq!(line.as_str(), "");
        line.backspace();
        assert_eq!(line.as_str(), "");
    }
}

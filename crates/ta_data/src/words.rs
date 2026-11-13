//! Words the launcher shows in the player's language: an English text with `{name}`
//! placeholders and what fills them. `ta-tools i18n` collects the texts marked [`en`] (and the
//! first argument of [`enf`]) from the mod check's sources into the languages' files; ta_data has
//! no interface, so the launcher translates a text and what fills it in turn ([`Words::render`]
//! with its `t`), and English is each text as it is ([`Words::english`]).

use serde::{Deserialize, Serialize};

/// Marks an English text the interface translates (`ta-tools i18n` collects it): the text.
pub const fn en(text: &'static str) -> &'static str {
    text
}

/// Words with placeholders: the English text (collected for translation) and what fills each
/// `{name}` in it.
pub fn enf<const N: usize>(text: &'static str, fills: [(&'static str, Fill); N]) -> Words {
    Words { text: text.into(), fills: fills.into_iter().map(|(name, fill)| (name.to_string(), fill)).collect(), raw: false }
}

/// A finding's words, or a part of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Words {
    /// The English text: a key of the languages' files, unless `raw`.
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<(String, Fill)>,
    /// Not translated: words the file readers give (an archive's or a model's fault, in their
    /// own words), shown as they are.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub raw: bool,
}

/// What fills a placeholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    /// Shown as it is: a unit's, a file's or an archive's name, a number.
    Name(String),
    /// Words of their own, translated in turn.
    Words(Words),
    /// A list, each item told once (with how many times, where more: TA: Escalation's menus name
    /// a unit several times over, to weight the computer players' choice), joined as the
    /// language joins one ("A, B and C"). Every item is named: a finding is told inherited by its
    /// English words (`validation::compare`), and two lists alike in their first items are not
    /// alike.
    List(Vec<Fill>),
    /// Alternatives, joined as the language offers a choice ("A, B or C"): the folders the game
    /// reads a kind of file from, the functions a script lacks.
    Either(Vec<Fill>),
}

impl Words {
    /// Words without placeholders.
    pub fn text(text: &'static str) -> Words {
        Words { text: text.into(), ..Words::default() }
    }

    /// Words shown as they are, in every language (see [`Words::raw`]).
    pub fn raw(text: impl Into<String>) -> Words {
        Words { text: text.into(), fills: Vec::new(), raw: true }
    }

    /// No words: a finding read from a report kept by an older version, before findings had them.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// In English.
    pub fn english(&self) -> String {
        self.render(&|text: &str| text.to_string())
    }

    /// In a language: `translate` gives a text's words in it (the English where it has none).
    pub fn render(&self, translate: &dyn Fn(&str) -> String) -> String {
        let text = if self.raw { self.text.clone() } else { translate(&self.text) };
        let fills: Vec<(&str, String)> = self.fills.iter().map(|(name, fill)| (name.as_str(), fill.render(translate))).collect();
        fill_in(&text, &fills)
    }

    /// Every text these words are made of, for translation: theirs, then those filling them.
    pub fn texts(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.gather(&mut out);
        out
    }

    fn gather<'a>(&'a self, out: &mut Vec<&'a str>) {
        if !self.raw {
            out.push(&self.text);
        }
        for (_, fill) in &self.fills {
            fill.gather(out);
        }
    }
}

impl From<Words> for Fill {
    fn from(words: Words) -> Fill {
        Fill::Words(words)
    }
}

impl Fill {
    /// A name, shown as it is.
    pub fn name(name: impl Into<String>) -> Fill {
        Fill::Name(name.into())
    }

    /// A number, as it is.
    pub fn number(n: impl std::fmt::Display) -> Fill {
        Fill::Name(n.to_string())
    }

    fn render(&self, translate: &dyn Fn(&str) -> String) -> String {
        match self {
            Fill::Name(name) => name.clone(),
            Fill::Words(words) => words.render(translate),
            Fill::List(items) => {
                let mut unique: Vec<(&Fill, usize)> = Vec::new();
                for item in items {
                    match unique.iter_mut().find(|(seen, _)| *seen == item) {
                        Some((_, n)) => *n += 1,
                        None => unique.push((item, 1)),
                    }
                }
                let shown: Vec<String> = unique
                    .iter()
                    .map(|(item, n)| {
                        let what = item.render(translate);
                        // (A list's item named more than once: "ARMBRTHA build option (3 times)".)
                        if *n > 1 { fill_in(&translate(en("{what} ({count} times)")), &[("what", what), ("count", n.to_string())]) } else { what }
                    })
                    .collect();
                join(&shown, translate, en("{list} and {item}"))
            }
            Fill::Either(items) => {
                let shown: Vec<String> = items.iter().map(|item| item.render(translate)).collect();
                join(&shown, translate, en("{list} or {item}"))
            }
        }
    }

    fn gather<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Fill::Name(_) => {}
            Fill::Words(words) => words.gather(out),
            Fill::List(items) | Fill::Either(items) => items.iter().for_each(|item| item.gather(out)),
        }
    }
}

/// Items joined as a list: "A", "A and B", "A, B and C" (`last_pattern`: how the last joins on,
/// "{list} and {item}" or "{list} or {item}"; the others after a comma, "{list}, {item}").
fn join(items: &[String], translate: &dyn Fn(&str) -> String, last_pattern: &str) -> String {
    let Some((first, rest)) = items.split_first() else { return String::new() };
    let Some((last, middle)) = rest.split_last() else { return first.clone() };
    let mut out = first.clone();
    for item in middle {
        out = fill_in(&translate(en("{list}, {item}")), &[("list", out), ("item", item.clone())]);
    }
    fill_in(&translate(last_pattern), &[("list", out), ("item", last.clone())])
}

/// `text` with each `{name}` of `fills` in it replaced, in one pass (what fills one is never read
/// for another's), anything else between braces kept as it is.
fn fill_in<S: AsRef<str>>(text: &str, fills: &[(&str, S)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let filled = after.find('}').and_then(|close| fills.iter().find(|(name, _)| *name == &after[..close]).map(|(_, value)| (close, value)));
        match filled {
            Some((close, value)) => {
                out.push_str(value.as_ref());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Fill {
        Fill::List(items.iter().map(|item| Fill::name(*item)).collect())
    }

    #[test]
    fn lists_are_told_whole_each_item_once() {
        let english = |items: &[&str]| enf("{items}", [("items", list(items))]).english();
        assert_eq!(english(&[]), "");
        assert_eq!(english(&["A"]), "A");
        assert_eq!(english(&["A", "B"]), "A and B");
        assert_eq!(english(&["A", "B", "C"]), "A, B and C");
        assert_eq!(english(&["A", "B", "C", "D", "E"]), "A, B, C, D and E", "every one named: compared by its words");
        assert_eq!(english(&["A", "B", "A", "A"]), "A (3 times) and B");
    }

    #[test]
    fn words_render_in_a_language_and_what_fills_them_too() {
        let label = |unit: &str| enf("{unit} build option", [("unit", Fill::name(unit))]);
        let words = enf("{whats}: {what} is missing.", [
            ("whats", Fill::List(vec![label("ARMCK").into(), label("ARMCV").into(), label("ARMCK").into()])),
            ("what", enf("unit {name}", [("name", Fill::name("ARMCHEAT"))]).into()),
        ]);
        assert_eq!(words.english(), "ARMCK build option (2 times) and ARMCV build option: unit ARMCHEAT is missing.");
        let german = |text: &str| match text {
            "{whats}: {what} is missing." => "{whats}: {what} fehlt.",
            "{unit} build option" => "Bauoption von {unit}",
            "unit {name}" => "Einheit {name}",
            "{what} ({count} times)" => "{what} ({count}-mal)",
            "{list} and {item}" => "{list} und {item}",
            other => other,
        }.to_string();
        assert_eq!(words.render(&german), "Bauoption von ARMCK (2-mal) und Bauoption von ARMCV: Einheit ARMCHEAT fehlt.");
        assert_eq!(words.texts(), ["{whats}: {what} is missing.", "{unit} build option", "{unit} build option", "{unit} build option", "unit {name}"]);
        // (Shown as they are: the file readers' words, and a name holding braces.)
        assert_eq!(Words::raw("bad {header}").render(&german), "bad {header}");
        assert_eq!(enf("{a} then {b}", [("a", Fill::name("{b}")), ("b", Fill::name("x"))]).english(), "{b} then x");
    }
}

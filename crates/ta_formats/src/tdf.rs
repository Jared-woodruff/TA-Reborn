//! Parser for TA's "TDF" text format, shared by `.tdf`, `.fbi`, `.gui`, `.ota` and more:
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=ARMCOM;      // comment
//!     [SubSection] { key = value; }
//! }
//! ```
//!
//! Keys and section names are case-insensitive. The parser is deliberately forgiving:
//! shipped data and decades of community mods contain missing semicolons, stray braces and
//! unterminated comments, and the original game tolerated most of them.

/// A `[name] { ... }` block. Order and duplicates are preserved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TdfSection {
    pub name: String,
    pub entries: Vec<(String, String)>,
    pub sections: Vec<TdfSection>,
}

impl TdfSection {
    /// Case-insensitive lookup of a value (the last assignment wins, as in TA).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    /// Parses an integer leniently: accepts leading `+`, trailing junk and float syntax;
    /// numbers past the `i64` range saturate.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(parse_int)
    }

    pub fn get_i64_or(&self, key: &str, default: i64) -> i64 {
        self.get_i64(key).unwrap_or(default)
    }

    /// [`TdfSection::get_i64_or`] clamped to the `i32` range (for values that must fit one).
    pub fn get_i32_or(&self, key: &str, default: i32) -> i32 {
        self.get_i64(key).map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32).unwrap_or(default)
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(parse_float)
    }

    pub fn get_f64_or(&self, key: &str, default: f64) -> f64 {
        self.get_f64(key).unwrap_or(default)
    }

    /// [`TdfSection::get_f64_or`] narrowed to `f32` (presentation values: past 16,777,216
    /// whole numbers lose precision, so game data takes `get_f64_or`).
    pub fn get_f32_or(&self, key: &str, default: f32) -> f32 {
        self.get_f64(key).map(|v| v as f32).unwrap_or(default)
    }

    /// TA booleans are integers; any non-zero value is true.
    pub fn get_bool_or(&self, key: &str, default: bool) -> bool {
        self.get_i64(key).map(|v| v != 0).unwrap_or(default)
    }

    /// Case-insensitive lookup of the first child section with this name.
    pub fn section(&self, name: &str) -> Option<&TdfSection> {
        self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Resolves a `/`-separated path of nested section names.
    pub fn section_path(&self, path: &str) -> Option<&TdfSection> {
        path.split('/').try_fold(self, |s, part| s.section(part))
    }

    /// An empty section.
    pub fn new(name: impl Into<String>) -> TdfSection {
        TdfSection { name: name.into(), ..Default::default() }
    }

    /// Sets a value: replaces every assignment of `key` (case-insensitive) by one, kept at
    /// the place of the first, or appends it.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.entries.iter().position(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(i) => {
                self.entries[i].1 = value;
                let mut n = 0;
                self.entries.retain(|(k, _)| {
                    n += 1;
                    n - 1 <= i || !k.eq_ignore_ascii_case(key)
                });
            }
            None => self.entries.push((key.to_string(), value)),
        }
    }

    /// The first child section with this name (case-insensitive), created at the end if
    /// missing.
    pub fn section_mut(&mut self, name: &str) -> &mut TdfSection {
        match self.sections.iter().position(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(i) => &mut self.sections[i],
            None => {
                self.sections.push(TdfSection::new(name));
                self.sections.last_mut().expect("just pushed")
            }
        }
    }

    fn write(&self, depth: usize, out: &mut String) {
        let indent = "\t".repeat(depth);
        out.push_str(&format!("{indent}[{}]\r\n{indent}\t{{\r\n", clean(&self.name, &[']'])));
        for (k, v) in &self.entries {
            out.push_str(&format!("{indent}\t{}={};\r\n", clean(k, &['=', '[', '{', '}']), clean(v, &[])));
        }
        for s in &self.sections {
            s.write(depth + 1, out);
        }
        out.push_str(&format!("{indent}\t}}\r\n"));
    }
}

/// A key, value or section name made safe to write: no line breaks, semicolons (they end a
/// value), `//` (starts a comment, except after `:` as in URLs) or `extra` characters.
fn clean(text: &str, extra: &[char]) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        if c == ';' || c == '\r' || c == '\n' || extra.contains(&c) || c == '/' && out.ends_with('/') && !out.ends_with(":/") {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// A parsed TDF document: a list of top-level sections.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TdfDocument {
    pub sections: Vec<TdfSection>,
}

impl TdfDocument {
    pub fn section(&self, name: &str) -> Option<&TdfSection> {
        self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Resolves `A/B/C` from the top level.
    pub fn section_path(&self, path: &str) -> Option<&TdfSection> {
        let mut parts = path.split('/');
        let first = self.section(parts.next()?)?;
        parts.try_fold(first, |s, part| s.section(part))
    }

    /// The document as TDF text in TA's own layout: tab-indented `[Section]` blocks with the
    /// braces on their own lines, `key=value;` entries, CRLF line ends. Anything that would
    /// not read back the same (line breaks, semicolons, comment starts) is dropped from keys
    /// and values, so `parse(&doc.to_text())` gives `doc` back for every parsed document.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for s in &self.sections {
            s.write(0, &mut out);
            out.push_str("\r\n");
        }
        out
    }

    /// [`TdfDocument::to_text`] as Latin-1 bytes, the encoding TA reads; text with letters
    /// Latin-1 lacks (a map named in Cyrillic or Chinese) as UTF-8 after its byte order mark,
    /// which [`parse_bytes`] reads back (TA's own files never begin with one).
    pub fn to_bytes(&self) -> Vec<u8> {
        let text = self.to_text();
        match text.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect::<Option<Vec<u8>>>() {
            Some(latin) => latin,
            None => [UTF8_BOM.as_slice(), text.as_bytes()].concat(),
        }
    }
}

/// The byte order mark of UTF-8 text.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Parses TDF text from raw bytes: Windows-1252 decoded as Latin-1, or UTF-8 after a byte
/// order mark (as [`TdfDocument::to_bytes`] writes text Latin-1 can't hold).
pub fn parse_bytes(bytes: &[u8]) -> TdfDocument {
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) && let Ok(text) = std::str::from_utf8(rest) {
        return parse(text);
    }
    parse_chars(bytes.iter().map(|&b| char::from(b)))
}

pub fn parse(text: &str) -> TdfDocument {
    parse_chars(text.chars())
}

fn parse_chars(chars: impl Iterator<Item = char>) -> TdfDocument {
    let mut p = Parser::new(chars);
    let mut root = TdfSection::default();
    p.parse_body(&mut root, true, 0);
    TdfDocument { sections: root.sections }
}

// Two-character lookahead is all the grammar needs. Decode Latin-1 or UTF-8 as
// consumed instead of allocating an intermediate String and Vec<char> per file.
struct Parser<I> {
    chars: I,
    current: Option<char>,
    next: Option<char>,
}

impl<I: Iterator<Item = char>> Parser<I> {
    fn new(mut chars: I) -> Self {
        Self { current: chars.next(), next: chars.next(), chars }
    }

    fn peek(&self) -> Option<char> {
        self.current
    }

    fn peek2(&self) -> Option<char> {
        self.next
    }

    fn advance(&mut self) {
        self.current = self.next;
        self.next = self.chars.next();
    }

    /// Skips whitespace and both comment styles.
    fn skip_trivia(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '\u{1a}' {
                self.advance();
            } else if c == '/' && self.peek2() == Some('/') {
                self.skip_line();
            } else if c == '/' && self.peek2() == Some('*') {
                self.advance(); self.advance();
                while self.peek().is_some() {
                    if self.peek() == Some('*') && self.peek2() == Some('/') {
                        self.advance(); self.advance();
                        break;
                    }
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.advance();
            if c == '\n' {
                break;
            }
        }
    }

    /// Parses entries and sub-sections until the closing brace (or EOF at top level).
    fn parse_body(&mut self, into: &mut TdfSection, top_level: bool, depth: usize) {
        // The tolerant parser discards only an excessive subtree, then resumes after
        // its matching brace. Never recurse or flatten hostile nesting into the root.
        if depth > 64 {
            self.skip_body();
            return;
        }
        loop {
            self.skip_trivia();
            let Some(c) = self.peek() else { return };
            match c {
                '[' => {
                    self.advance();
                    let mut name = String::new();
                    while let Some(c) = self.peek() {
                        if c == ']' || c == '\n' {
                            break;
                        }
                        name.push(c);
                        self.advance();
                    }
                    if self.peek() == Some(']') {
                        self.advance();
                    }
                    let mut section = TdfSection { name: name.trim().to_string(), ..Default::default() };
                    self.skip_trivia();
                    if self.peek() == Some('{') {
                        self.advance();
                        self.parse_body(&mut section, false, depth + 1);
                    }
                    into.sections.push(section);
                }
                '}' => {
                    self.advance();
                    if !top_level {
                        return;
                    }
                    // A stray closing brace at the top level: ignore it.
                }
                '{' => {
                    // Anonymous block (malformed); fold its contents into the current section.
                    self.advance();
                    self.parse_body(into, false, depth + 1);
                }
                ';' => self.advance(),
                _ => self.parse_entry(into),
            }
        }
    }

    fn skip_body(&mut self) {
        let mut nesting = 1usize;
        let mut discarded = TdfSection::default();
        while nesting != 0 {
            self.skip_trivia();
            match self.peek() {
                None => break,
                Some('{') => { self.advance(); nesting += 1; }
                Some('}') => { self.advance(); nesting -= 1; }
                Some('[') => {
                    self.advance();
                    while self.peek().is_some_and(|c| c != ']' && c != '\n') { self.advance(); }
                    if self.peek() == Some(']') { self.advance(); }
                }
                Some(';') => self.advance(),
                _ => { self.parse_entry(&mut discarded); discarded.entries.clear(); }
            }
        }
    }

    /// Parses `key = value;`. A value ends at `;`, a line break, or a `//` comment.
    fn parse_entry(&mut self, into: &mut TdfSection) {
        let mut key = String::new();
        while let Some(c) = self.peek() {
            match c {
                '=' => break,
                ';' | '\n' | '{' | '}' | '[' => {
                    // Not an assignment; discard the junk token.
                    if c == ';' || c == '\n' {
                        self.advance();
                    }
                    return;
                }
                _ => {
                    key.push(c);
                    self.advance();
                }
            }
        }
        if self.peek() != Some('=') {
            return;
        }
        self.advance();
        let mut value = String::new();
        while let Some(c) = self.peek() {
            match c {
                ';' => {
                    self.advance();
                    break;
                }
                '\n' | '\r' => break,
                '/' if self.peek2() == Some('/') && !value.ends_with(':') => break,
                _ => {
                    value.push(c);
                    self.advance();
                }
            }
        }
        let key = key.trim();
        if !key.is_empty() {
            into.entries.push((key.to_string(), value.trim().to_string()));
        }
    }
}

fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'-') | Some(b'+')) {
        end = 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        // Values like ".5" – fall back to float parsing and truncate like atoi would not;
        // TA used atoi, which yields 0 here.
        return parse_float(s).map(|f| f as i64);
    }
    match s[..end].parse::<i64>() {
        Ok(v) => Some(v),
        Err(e) => match e.kind() {
            std::num::IntErrorKind::PosOverflow => Some(i64::MAX),
            std::num::IntErrorKind::NegOverflow => Some(i64::MIN),
            _ => None,
        },
    }
}

fn parse_float(s: &str) -> Option<f64> {
    let s = s.trim();
    // Take the longest numeric prefix (atof semantics).
    let bytes = s.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'-') | Some(b'+')) {
        end = 1;
    }
    let mut seen_digit = false;
    let mut seen_dot = false;
    while end < bytes.len() {
        match bytes[end] {
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => break,
        }
        end += 1;
    }
    if !seen_digit {
        return None;
    }
    s[..end].parse().ok()
}

/// A problem in a TDF file's structure that [`parse`] silently works round ([`lint`]); its
/// words in English are its `Display`. (A section's name is as [`lint`] quotes it.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintProblem {
    /// A section's name runs to the line's end.
    NameUnclosed(String),
    /// A section's header without a body.
    NoBody(String),
    /// A `}` that closes nothing.
    StrayClose,
    /// An anonymous block never closed.
    BlockUnclosed,
    /// A section never closed: any section after it is read as part of it.
    SectionUnclosed(String),
    /// A `/*` comment never ended.
    CommentUnended,
}

impl std::fmt::Display for LintProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LintProblem::NameUnclosed(name) => write!(f, "The section name [{name} has no closing ]."),
            LintProblem::NoBody(name) => write!(f, "[{name}] has no {{ }} body after it: what follows is read into the section around it."),
            LintProblem::StrayClose => f.write_str("This } closes no section."),
            LintProblem::BlockUnclosed => f.write_str("A { opened on this line is never closed."),
            LintProblem::SectionUnclosed(name) => write!(f, "[{name}] opened on this line is never closed: any section after it is read as part of it."),
            LintProblem::CommentUnended => f.write_str("A /* comment opened on this line never ends: the rest of the file is read as a comment."),
        }
    }
}

/// Problems in a TDF file's structure that [`parse`] silently works round, with the line
/// each is on (1-based): a section never closed (any section after it is read as part of it),
/// a closing brace that closes nothing, a section header with no body or no `]`, and a
/// comment never ended. Follows the parser's grammar: brackets and braces inside a value are
/// text.
pub fn lint(bytes: &[u8]) -> Vec<(u32, LintProblem)> {
    let chars: Vec<char> = match bytes.strip_prefix(&UTF8_BOM).and_then(|rest| std::str::from_utf8(rest).ok()) {
        Some(text) => text.chars().collect(),
        None => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    let mut lint = Lint { chars, i: 0, line: 1, out: Vec::new() };
    let mut open: Vec<(String, u32)> = Vec::new();
    while let Some(c) = lint.at(0) {
        match c {
            '[' => {
                let start = lint.line;
                lint.i += 1;
                let mut name = String::new();
                while let Some(c) = lint.at(0) {
                    if c == ']' || c == '\n' {
                        break;
                    }
                    name.push(c);
                    lint.i += 1;
                }
                let name = shown(&name);
                if lint.at(0) == Some(']') {
                    lint.i += 1;
                } else {
                    // (The name ran to the line's end: nothing more to say of it.)
                    lint.out.push((start, LintProblem::NameUnclosed(name)));
                    continue;
                }
                lint.trivia();
                if lint.at(0) == Some('{') {
                    lint.i += 1;
                    open.push((name, start));
                } else {
                    lint.out.push((start, LintProblem::NoBody(name)));
                }
            }
            // An anonymous block: the parser folds it into the section around it.
            '{' => {
                open.push((String::new(), lint.line));
                lint.i += 1;
            }
            '}' => {
                if open.pop().is_none() {
                    lint.out.push((lint.line, LintProblem::StrayClose));
                }
                lint.i += 1;
            }
            ';' => lint.i += 1,
            c if c.is_whitespace() || c == '\u{1a}' || c == '/' && matches!(lint.at(1), Some('/') | Some('*')) => lint.trivia(),
            _ => {
                // An entry: its key up to `=`, then its value up to `;` or the line's end.
                while lint.at(0).is_some_and(|c| !matches!(c, '=' | ';' | '\n' | '{' | '}' | '[')) {
                    lint.i += 1;
                }
                if lint.at(0) == Some('=') {
                    lint.i += 1;
                    let mut last = '=';
                    while let Some(c) = lint.at(0) {
                        if c == ';' {
                            lint.i += 1;
                            break;
                        }
                        if c == '\n' || c == '\r' || c == '/' && lint.at(1) == Some('/') && last != ':' {
                            break;
                        }
                        last = c;
                        lint.i += 1;
                    }
                }
            }
        }
    }
    for (name, start) in open {
        lint.out.push((start, if name.is_empty() { LintProblem::BlockUnclosed } else { LintProblem::SectionUnclosed(name) }));
    }
    lint.out.sort_by_key(|(line, _)| *line);
    lint.out
}

/// A section name as a problem quotes it: trimmed, and cut short.
fn shown(name: &str) -> String {
    let name = name.trim();
    if name.chars().count() <= 40 { name.to_string() } else { format!("{}…", name.chars().take(40).collect::<String>()) }
}

struct Lint {
    chars: Vec<char>,
    i: usize,
    line: u32,
    out: Vec<(u32, LintProblem)>,
}

impl Lint {
    fn at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).copied()
    }

    /// Past whitespace and comments, counting lines.
    fn trivia(&mut self) {
        while let Some(c) = self.at(0) {
            if c == '\n' {
                self.line += 1;
                self.i += 1;
            } else if c.is_whitespace() || c == '\u{1a}' {
                self.i += 1;
            } else if c == '/' && self.at(1) == Some('/') {
                while self.at(0).is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
            } else if c == '/' && self.at(1) == Some('*') {
                let start = self.line;
                self.i += 2;
                loop {
                    match self.at(0) {
                        None => {
                            self.out.push((start, LintProblem::CommentUnended));
                            return;
                        }
                        Some('*') if self.at(1) == Some('/') => {
                            self.i += 2;
                            break;
                        }
                        Some('\n') => {
                            self.line += 1;
                            self.i += 1;
                        }
                        Some(_) => self.i += 1,
                    }
                }
            } else {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_finds_what_the_tolerant_parser_works_round_with_its_lines() {
        assert!(lint(b"[A]\n{\n  x=1; // c\n  [B] { y={not a section}; url=http://x/y; }\n}\n/* ok */").is_empty());
        let problems = lint(b"[WEAPON_A]\n{\n  range=100;\n\n[WEAPON_B]\n{\n  range=200;\n}\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].0, 1);
        assert!(problems[0].1.to_string().contains("[WEAPON_A] opened on this line is never closed"));
        let problems = lint(b"[A]{x=1;}\n}\n[B]\nx=2;\n[C{y=1;}\n/* open");
        let lines: Vec<u32> = problems.iter().map(|(l, _)| *l).collect();
        assert_eq!(lines, [2, 3, 5, 6], "{problems:?}");
        assert!(problems[0].1.to_string().contains("closes no section"));
        assert!(problems[1].1.to_string().contains("[B] has no"));
        assert!(problems[2].1.to_string().contains("[C{y=1;} has no closing ]"));
        assert!(problems[3].1.to_string().contains("never ends"));
    }

    #[test]
    fn streaming_latin1_and_unicode_keep_text_and_tolerant_syntax() {
        // Includes every byte value, Latin-1 whitespace, URLs, comments, malformed
        // tokens, missing terminators and nested sections in both input paths.
        let mut bytes = b"[\xe9t\xe9]{name=caf\xe9; url=http://x/y;/*c*/[child]{x=1;} raw=".to_vec();
        bytes.extend(0..=255);
        bytes.extend_from_slice(b"; } // trailing\n[tail]{a=2");
        assert_eq!(parse_bytes(&bytes), parse(&crate::bin::latin1(&bytes)));
        let text = "[世界]{名=日本語; url=https://x/y;[子]{値=✓;} }";
        let doc = parse(text);
        assert_eq!(doc.section("世界").unwrap().get("名"), Some("日本語"));
        assert_eq!(doc.section_path("世界/子").unwrap().get("値"), Some("✓"));
        for text in ["/", "/*", "//", "[", "{", "x=", "[A]{x=http://", "[A]{x=1;}/*x"] {
            assert_eq!(parse_bytes(text.as_bytes()), parse(text));
        }
    }

    #[test]
    fn text_latin1_lacks_is_written_as_utf8_and_read_back() {
        let mut doc = TdfDocument::default();
        let mut header = TdfSection::new("GlobalHeader");
        header.set("missionname", "Café");
        doc.sections.push(header);
        let latin = doc.to_bytes();
        assert!(latin.contains(&0xE9) && !latin.starts_with(&UTF8_BOM), "Latin-1 text stays as TA writes it");
        assert_eq!(parse_bytes(&latin).section("GlobalHeader").unwrap().get("missionname"), Some("Café"));
        doc.sections[0].set("missionname", "Пустыня 沙漠");
        let utf8 = doc.to_bytes();
        assert!(utf8.starts_with(&UTF8_BOM));
        assert_eq!(parse_bytes(&utf8).section("GlobalHeader").unwrap().get("missionname"), Some("Пустыня 沙漠"));
    }

    #[test]
    fn parses_nested_sections_and_comments() {
        let doc = parse(
            "// header\n[UNITINFO]\n{\n  UnitName=ARMCOM; // the commander\n  Name = Arm Commander ;\n\
             /* block\n comment */ BuildCostMetal=2500;\n  [Sub]{ a=1; }\n}\n",
        );
        let unit = doc.section("unitinfo").unwrap();
        assert_eq!(unit.get("unitname"), Some("ARMCOM"));
        assert_eq!(unit.get("Name"), Some("Arm Commander"));
        assert_eq!(unit.get_i64("buildcostmetal"), Some(2500));
        assert_eq!(unit.section("SUB").unwrap().get("a"), Some("1"));
    }

    #[test]
    fn tolerates_missing_semicolons_and_braces() {
        let doc = parse("[A]\n{\n x=1\n y=2;\n [B]\n { z=3;\n}\n");
        let a = doc.section("A").unwrap();
        assert_eq!(a.get("x"), Some("1"));
        assert_eq!(a.get("y"), Some("2"));
        assert_eq!(a.section("B").unwrap().get("z"), Some("3"));
    }

    #[test]
    fn lenient_numbers() {
        assert_eq!(parse_int("12abc"), Some(12));
        assert_eq!(parse_int(" -7 "), Some(-7));
        assert_eq!(parse_float("0.25x"), Some(0.25));
        assert_eq!(parse_float("abc"), None);
    }

    #[test]
    fn big_numbers_keep_their_precision() {
        let doc = parse("[A]{ cost=123456789012; time=16777217; huge=99999999999999999999999; neg=-99999999999999999999999; }");
        let a = doc.section("A").unwrap();
        assert_eq!(a.get_i64_or("cost", 0), 123_456_789_012);
        assert_eq!(a.get_f64_or("cost", 0.0), 123_456_789_012.0);
        assert_eq!(a.get_f64_or("time", 0.0), 16_777_217.0, "one past what an f32 holds exactly");
        assert_eq!(a.get_i64_or("huge", 0), i64::MAX, "saturates");
        assert_eq!(a.get_i64_or("neg", 0), i64::MIN);
        assert_eq!(a.get_i32_or("cost", 0), i32::MAX);
        assert_eq!(a.get_i64_or("missing", 7), 7);
    }

    #[test]
    fn keeps_urls_intact() {
        let doc = parse("[A]{ url=http://example.com/x; }");
        assert_eq!(doc.section("A").unwrap().get("url"), Some("http://example.com/x"));
    }
}

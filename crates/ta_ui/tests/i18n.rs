//! Switching the interface's language. The language is the whole process's, so this test runs
//! in a process of its own: among the library's unit tests, which run side by side on threads
//! of one process and read their text in English, it turned some of their text into the test
//! language's or the pseudo-language's while it ran ("[2 / 10 h]" for "2 / 10 h"). One test,
//! its cases in order, for the same reason.

use ta_ui::i18n::{PSEUDO_CODE, language, languages, set_language, t, tf};

#[test]
fn languages_translate_and_fall_back_to_english() {
    let dir = std::env::temp_dir().join(format!("ta_ui_i18n_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("xx.toml"), "[language]\nname = \"Test\"\n[strings]\n\"SKIRMISH\" = \"GEFECHT\"\n\"{n} units\" = \"{n} Einheiten\"\n\"EMPTY\" = \"\"\n").unwrap();
    std::fs::write(dir.join("template.toml"), "[strings]\n").unwrap();
    let dirs = vec![dir.clone()];
    let langs = languages(&dirs);
    assert_eq!(langs[0].0, "en");
    assert!(langs.iter().any(|(c, n)| c == "xx" && n == "Test"));
    assert!(!langs.iter().any(|(c, _)| c == "template"));

    set_language("xx", &dirs).unwrap();
    assert_eq!(t("SKIRMISH"), "GEFECHT");
    assert_eq!(t("CAMPAIGN"), "CAMPAIGN");
    assert_eq!(t("EMPTY"), "EMPTY");
    assert_eq!(tf("{n} units", &[("n", &12)]), "12 Einheiten");
    assert_eq!(language(), "xx");

    set_language(PSEUDO_CODE, &dirs).unwrap();
    assert_eq!(t("CAMPAIGN"), "[CÁMPÁÍGN]");
    assert_eq!(tf("Range {range}", &[("range", &300)]), "[Rángé 300]");

    assert!(set_language("nope", &dirs).is_err());
    assert_eq!(t("SKIRMISH"), "SKIRMISH");
    set_language("", &dirs).unwrap();
    assert_eq!(language(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

//! Translated strings, with Fluent.
//!
//! Self-contained so other apps can copy it: nothing in this file is Legible's. The
//! one app-specific part is [`embedded`], which lists the `.ftl` files compiled into the exe.
//!
//! ```
//! use legible_core::{i18n, tr};
//!
//! i18n::init("en-GB");
//! assert_eq!(i18n::tr("app-name"), "Legible");
//! assert_eq!(tr!("error-file-not-found", file = "notes.md"), "There's no file called notes.md.");
//! ```
//!
//! A key that is missing in every language, or a message that fails to format, is logged with
//! `tracing` and shows as the key itself, never as an empty string. Tests catch both first.

pub mod embedded;
mod plurals;

use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, OnceLock, RwLock};

use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
pub use fluent_bundle::{FluentArgs, FluentValue};
use unic_langid::LanguageIdentifier;

pub use plurals::PluralForms;

/// One language's strings: its tag and its `.ftl` files, as (file name, contents).
pub struct Locale {
    pub tag: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

/// A problem in a locale's files: a syntax error, or a key defined twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleError {
    pub locale: String,
    pub file: String,
    pub detail: String,
}

impl fmt::Display for LocaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}: {}", self.locale, self.file, self.detail)
    }
}

/// Looks strings up in a chain of languages: the one asked for, then the fallback.
pub struct Localizer {
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Localizer {
    /// Builds a localizer for `requested` (a tag such as `en-GB` or `fr_FR`), falling back to
    /// `fallback`. Errors in the files are logged; the rest of the strings still load.
    pub fn new(locales: &[Locale], requested: &str, fallback: &str) -> Self {
        let (localizer, errors) = Self::build(locales, requested, fallback);
        for error in errors {
            tracing::error!(%error, "a string file has an error");
        }
        localizer
    }

    /// As [`Localizer::new`], returning the files' errors instead of logging them.
    pub fn build(locales: &[Locale], requested: &str, fallback: &str) -> (Self, Vec<LocaleError>) {
        let mut errors = Vec::new();
        let bundles = negotiate(locales, requested, fallback)
            .into_iter()
            .map(|locale| bundle(locale, &mut errors))
            .collect();
        (Self { bundles }, errors)
    }

    /// The tags of the languages looked in, in order.
    pub fn languages(&self) -> Vec<String> {
        self.bundles
            .iter()
            .map(|b| b.locales[0].to_string())
            .collect()
    }

    /// Whether some language in the chain has `key`.
    pub fn has(&self, key: &str) -> bool {
        self.bundles.iter().any(|b| has_value(b, key))
    }

    /// The string for `key`.
    pub fn tr(&self, key: &str) -> String {
        self.format(key, None)
    }

    /// The string for `key`, with its `{ $name }` placeables filled from `args`.
    pub fn tr_args(&self, key: &str, args: &FluentArgs) -> String {
        self.format(key, Some(args))
    }

    /// The string for `key` in each plural form of the language it's found in, as chosen by the
    /// number `arg`, with each `{ $arg }` kept as `{arg}`: for a script that counts for itself,
    /// fills in the number as it formats it, and picks the form ([`PluralForms`]).
    pub fn tr_plurals(&self, key: &str, arg: &str) -> PluralForms {
        let found = self
            .bundles
            .iter()
            .find_map(|b| Some((b, b.get_message(key)?.value()?)));
        let Some((bundle, pattern)) = found else {
            tracing::warn!(key, "a string is missing in every language");
            let language = self.languages().into_iter().next().unwrap_or_default();
            return PluralForms::only(language, key);
        };
        plurals::forms(bundle, key, pattern, arg)
    }

    fn format(&self, key: &str, args: Option<&FluentArgs>) -> String {
        for bundle in &self.bundles {
            if let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) {
                return format_pattern(bundle, key, pattern, args);
            }
        }
        tracing::warn!(key, "a string is missing in every language");
        key.to_owned()
    }
}

/// `pattern`, `key`'s or made from it, formatted in `bundle`'s language; a problem is logged.
fn format_pattern(
    bundle: &FluentBundle<FluentResource>,
    key: &str,
    pattern: &fluent_syntax::ast::Pattern<&str>,
    args: Option<&FluentArgs>,
) -> String {
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);
    if !errors.is_empty() {
        tracing::warn!(key, language = %bundle.locales[0], ?errors, "a string didn't format cleanly");
    }
    text.into_owned()
}

fn has_value(bundle: &FluentBundle<FluentResource>, key: &str) -> bool {
    bundle.get_message(key).is_some_and(|m| m.value().is_some())
}

fn bundle(locale: &Locale, errors: &mut Vec<LocaleError>) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = locale.tag.parse().unwrap_or_default();
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    // No Unicode isolation marks around placeables: they show as stray characters in consoles
    // and logs. A right-to-left language may want them back, per bundle.
    bundle.set_use_isolating(false);
    for (file, source) in locale.files {
        let mut problem = |detail: String| {
            errors.push(LocaleError {
                locale: locale.tag.to_owned(),
                file: (*file).to_owned(),
                detail,
            })
        };
        let resource = match FluentResource::try_new((*source).to_owned()) {
            Ok(resource) => resource,
            Err((resource, parse_errors)) => {
                parse_errors.iter().for_each(|e| problem(e.to_string()));
                resource
            }
        };
        if let Err(add_errors) = bundle.add_resource(resource) {
            add_errors.iter().for_each(|e| problem(e.to_string()));
        }
    }
    bundle
}

/// The locales to look in, best first: an exact match for `requested`, then the same language
/// without a region, then any region of it, then `fallback`.
fn negotiate<'a>(locales: &'a [Locale], requested: &str, fallback: &str) -> Vec<&'a Locale> {
    let parse = |tag: &str| tag.replace('_', "-").parse::<LanguageIdentifier>().ok();
    let ids: Vec<(&Locale, Option<LanguageIdentifier>)> =
        locales.iter().map(|l| (l, parse(l.tag))).collect();
    let find = |matches: &dyn Fn(&LanguageIdentifier) -> bool| {
        ids.iter()
            .find(|(_, id)| id.as_ref().is_some_and(matches))
            .map(|(l, _)| *l)
    };
    let mut found = Vec::new();
    if let Some(wanted) = parse(requested) {
        found.push(find(&|id| *id == wanted));
        found.push(find(&|id| {
            id.language == wanted.language && id.region.is_none()
        }));
        found.push(find(&|id| id.language == wanted.language));
    }
    found.push(locales.iter().find(|l| l.tag == fallback));
    let mut chain: Vec<&Locale> = Vec::new();
    for locale in found.into_iter().flatten() {
        if !chain.iter().any(|c| std::ptr::eq(*c, locale)) {
            chain.push(locale);
        }
    }
    chain
}

fn global() -> &'static RwLock<Arc<Localizer>> {
    static GLOBAL: OnceLock<RwLock<Arc<Localizer>>> = OnceLock::new();
    GLOBAL.get_or_init(|| RwLock::new(Arc::new(embedded_localizer(embedded::FALLBACK))))
}

fn embedded_localizer(requested: &str) -> Localizer {
    Localizer::new(embedded::LOCALES, requested, embedded::FALLBACK)
}

fn current() -> Arc<Localizer> {
    Arc::clone(&global().read().unwrap_or_else(|e| e.into_inner()))
}

/// Picks the language for [`tr`] and [`tr_args`], from the embedded strings. Until it's called,
/// they use the fallback language.
pub fn init(requested: &str) {
    let localizer = Arc::new(embedded_localizer(requested));
    *global().write().unwrap_or_else(|e| e.into_inner()) = localizer;
}

/// The tags of the languages [`tr`] looks in, in order.
pub fn languages() -> Vec<String> {
    current().languages()
}

/// The string for `key` in the language picked by [`init`].
pub fn tr(key: &str) -> String {
    current().tr(key)
}

/// The string for `key`, with its placeables filled from `args`. [`tr!`](crate::tr) is shorter.
pub fn tr_args(key: &str, args: &FluentArgs) -> String {
    current().tr_args(key, args)
}

/// The string for `key` in each plural form of its language: [`Localizer::tr_plurals`].
pub fn tr_plurals(key: &str, arg: &str) -> PluralForms {
    current().tr_plurals(key, arg)
}

/// Builds Fluent arguments from (name, value) pairs.
pub fn args<'a>(pairs: impl IntoIterator<Item = (&'a str, FluentValue<'a>)>) -> FluentArgs<'a> {
    let mut args = FluentArgs::new();
    for (name, value) in pairs {
        args.set(Cow::Borrowed(name), value);
    }
    args
}

/// `tr!("key")`, or `tr!("key", name = value, ...)` with any value a [`FluentValue`] converts
/// from (text and numbers).
#[macro_export]
macro_rules! tr {
    ($key:expr $(,)?) => {
        $crate::i18n::tr($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr_args(
            $key,
            &$crate::i18n::args([$((stringify!($name), $crate::i18n::FluentValue::from($value))),+]),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCALES: &[Locale] = &[
        Locale {
            tag: "en",
            files: &[(
                "a.ftl",
                "hello = Hello, { $name }!\n\
                 only-en = English only\n\
                 count =\n    { $n ->\n        [one] one thing\n       *[other] { $n } things\n    }\n",
            )],
        },
        Locale {
            tag: "fr",
            files: &[(
                "a.ftl",
                "hello = Bonjour, { $name } !\n\
                 count =\n    { $n ->\n        [one] { $n } chose\n       *[other] { $n } choses\n    }\n",
            )],
        },
        Locale {
            tag: "pt-BR",
            files: &[("a.ftl", "hello = Olá, { $name }!\n")],
        },
    ];

    fn localizer(requested: &str) -> Localizer {
        let (localizer, errors) = Localizer::build(TEST_LOCALES, requested, "en");
        assert!(errors.is_empty(), "{errors:?}");
        localizer
    }

    #[test]
    fn negotiates_exact_base_region_then_fallback() {
        assert_eq!(localizer("fr").languages(), ["fr", "en"]);
        assert_eq!(localizer("fr-CA").languages(), ["fr", "en"]);
        assert_eq!(localizer("fr_FR").languages(), ["fr", "en"]);
        assert_eq!(localizer("pt-PT").languages(), ["pt-BR", "en"]);
        assert_eq!(localizer("de-DE").languages(), ["en"]);
        assert_eq!(localizer("en-GB").languages(), ["en"]);
        assert_eq!(localizer("not a tag!").languages(), ["en"]);
    }

    #[test]
    fn formats_arguments_and_plurals() {
        let en = localizer("en");
        assert_eq!(
            en.tr_args("hello", &args([("name", "Ada".into())])),
            "Hello, Ada!"
        );
        assert_eq!(en.tr_args("count", &args([("n", 1.into())])), "one thing");
        assert_eq!(en.tr_args("count", &args([("n", 3.into())])), "3 things");
    }

    #[test]
    fn plural_forms_fall_back_per_key() {
        // French's own, then English's for a key French hasn't got (the forms: plurals.rs).
        let fr = localizer("fr").tr_plurals("count", "n");
        assert_eq!(fr.language, "fr");
        assert!(fr.forms.contains(&("one", "{n} chose".to_owned())));
        assert_eq!(localizer("fr").tr_plurals("only-en", "n").language, "en");
        // Missing everywhere: the key, in every form.
        let missing = localizer("en").tr_plurals("no-such-key", "n");
        assert!(missing.forms.iter().all(|(_, text)| text == "no-such-key"));
        assert_eq!(missing.forms.len(), 6);
    }

    #[test]
    fn falls_back_per_key() {
        let fr = localizer("fr");
        assert_eq!(
            fr.tr_args("hello", &args([("name", "Ada".into())])),
            "Bonjour, Ada !"
        );
        assert_eq!(fr.tr("only-en"), "English only");
    }

    #[test]
    fn a_missing_key_shows_as_itself() {
        let en = localizer("en");
        assert!(!en.has("no-such-key"));
        assert_eq!(en.tr("no-such-key"), "no-such-key");
    }

    #[test]
    fn a_missing_argument_still_shows_the_rest() {
        let en = localizer("en");
        assert_eq!(en.tr("hello"), "Hello, {$name}!");
    }

    #[test]
    fn reports_syntax_errors_and_duplicates() {
        static BAD: &[Locale] = &[Locale {
            tag: "en",
            files: &[
                ("bad.ftl", "good = Fine\nbroken = { $\n"),
                ("dup.ftl", "good = Again\n"),
            ],
        }];
        let (localizer, errors) = Localizer::build(BAD, "en", "en");
        let files: Vec<_> = errors.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(files, ["bad.ftl", "dup.ftl"]);
        assert_eq!(localizer.tr("good"), "Fine");
    }

    #[test]
    fn the_macro_takes_text_and_numbers() {
        assert_eq!(crate::tr!("app-name"), "Legible");
        let file = String::from("notes.md");
        assert_eq!(
            crate::tr!("error-file-not-found", file = file.as_str()),
            "There's no file called notes.md."
        );
        assert_eq!(crate::tr!("chrome-status-words", count = 1), "1 word");
        assert_eq!(crate::tr!("chrome-status-words", count = 3), "3 words");
    }
}

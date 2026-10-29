//! A message in each plural form, for a script that counts for itself ([`Localizer::tr_plurals`]
//! (super::Localizer::tr_plurals)).
//!
//! Fluent picks a message's variant by the count, then writes the count in. A script has the
//! count and `Intl.PluralRules`, but not Fluent: so the message is resolved here once for each
//! form, from its own syntax, with the count's places kept for the script. A select on the
//! count takes the variant Fluent would for a count in that form, and each `{ $count }` becomes
//! the placeholder itself, so neither a digit in the text nor a second use of the count is ever
//! mistaken for the number. Variants for exact numbers (`[0]`) are forms of their own.
//!
//! The count is supported as a selector and as a placeable. Anywhere else (an argument to
//! `NUMBER()`, say) it's missing, which Fluent logs, and shows as `{$count}`.

use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
use fluent_syntax::ast::{
    Expression, Identifier, InlineExpression, Pattern, PatternElement, Variant, VariantKey,
};

use super::{FluentArgs, format_pattern};

/// CLDR's cardinal plural categories, as Fluent's variant keys and `Intl.PluralRules` name them.
const CATEGORIES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// A message in each plural form of its language, for a script to choose from: the form for an
/// exact number if there is one, else the one for the number's category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluralForms {
    /// The tag of the language the forms are in, for `Intl.PluralRules`.
    pub language: String,
    /// The message for a count in each of CLDR's categories, all six: a category the language
    /// doesn't use has the default variant.
    pub forms: Vec<(&'static str, String)>,
    /// The message for each number a variant is for exactly (`[0]`), as a script writes the
    /// number (`String(n)`), as Fluent picks for that number: it goes before the category's.
    pub exact: Vec<(String, String)>,
}

impl PluralForms {
    /// `text` in every form: a message that's missing shows as its key.
    pub(super) fn only(language: String, text: &str) -> PluralForms {
        PluralForms {
            language,
            forms: CATEGORIES.map(|c| (c, text.to_owned())).to_vec(),
            exact: Vec::new(),
        }
    }
}

/// Which variant a select on the count takes.
#[derive(Clone, Copy)]
enum Choice<'c> {
    /// For a count in this category that no variant names exactly.
    Category(&'c str),
    /// For this exact number, in this category.
    Exact(f64, &'c str),
}

impl Choice<'_> {
    /// Whether a variant with `key` is taken: the first that is, as Fluent goes in order.
    fn takes(self, key: &VariantKey<&str>) -> bool {
        match (self, key) {
            (
                Choice::Category(category) | Choice::Exact(_, category),
                VariantKey::Identifier { name },
            ) => *name == category,
            (Choice::Exact(n, _), VariantKey::NumberLiteral { value }) => {
                value.parse::<f64>() == Ok(n)
            }
            (Choice::Category(_), VariantKey::NumberLiteral { .. }) => false,
        }
    }
}

/// `key`'s `pattern`, in `bundle`, in each plural form of the count `arg`.
pub(super) fn forms(
    bundle: &FluentBundle<FluentResource>,
    key: &str,
    pattern: &Pattern<&str>,
    arg: &str,
) -> PluralForms {
    let placeholder = format!("{{{arg}}}");
    let form = |choice| {
        let chosen = choose(pattern, arg, &placeholder, choice);
        format_pattern(bundle, key, &chosen, None)
    };
    let forms = CATEGORIES.map(|c| (c, form(Choice::Category(c)))).to_vec();
    let mut numbers = Vec::new();
    exact_numbers(pattern, arg, &mut numbers);
    let exact = numbers
        .into_iter()
        .map(|n| {
            (
                n.to_string(),
                form(Choice::Exact(n, category(bundle, arg, n))),
            )
        })
        .collect();
    PluralForms {
        language: bundle.locales[0].to_string(),
        forms,
        exact,
    }
}

/// `pattern` with each select on `arg` resolved as `choice` says, and each `{ $arg }` as
/// `placeholder`.
fn choose<'a>(
    pattern: &Pattern<&'a str>,
    arg: &str,
    placeholder: &'a str,
    choice: Choice,
) -> Pattern<&'a str> {
    let mut elements = Vec::new();
    for element in &pattern.elements {
        match element {
            PatternElement::Placeable { expression } => {
                place(expression, arg, placeholder, choice, &mut elements)
            }
            text => elements.push(text.clone()),
        }
    }
    Pattern { elements }
}

/// `expression`, resolved as [`choose`] does, onto `out`.
fn place<'a>(
    expression: &Expression<&'a str>,
    arg: &str,
    placeholder: &'a str,
    choice: Choice,
    out: &mut Vec<PatternElement<&'a str>>,
) {
    match expression {
        Expression::Select { selector, variants } if is_arg(selector, arg) => {
            let variant = variants
                .iter()
                .find(|v| choice.takes(&v.key))
                .or_else(|| variants.iter().find(|v| v.default));
            if let Some(variant) = variant {
                out.extend(choose(&variant.value, arg, placeholder, choice).elements);
            }
        }
        // A select on something else: each of its variants resolved in turn.
        Expression::Select { selector, variants } => out.push(PatternElement::Placeable {
            expression: Expression::Select {
                selector: selector.clone(),
                variants: variants
                    .iter()
                    .map(|v| Variant {
                        key: v.key.clone(),
                        value: choose(&v.value, arg, placeholder, choice),
                        default: v.default,
                    })
                    .collect(),
            },
        }),
        Expression::Inline(inline) if is_arg(inline, arg) => {
            out.push(PatternElement::TextElement { value: placeholder })
        }
        Expression::Inline(InlineExpression::Placeable { expression }) => {
            place(expression, arg, placeholder, choice, out)
        }
        other => out.push(PatternElement::Placeable {
            expression: other.clone(),
        }),
    }
}

fn is_arg(expression: &InlineExpression<&str>, arg: &str) -> bool {
    matches!(expression, InlineExpression::VariableReference { id } if id.name == arg)
}

/// The numbers the variants of selects on `arg` in `pattern` are for exactly, once each.
fn exact_numbers(pattern: &Pattern<&str>, arg: &str, out: &mut Vec<f64>) {
    for element in &pattern.elements {
        if let PatternElement::Placeable { expression } = element {
            exact_numbers_in(expression, arg, out);
        }
    }
}

fn exact_numbers_in(expression: &Expression<&str>, arg: &str, out: &mut Vec<f64>) {
    match expression {
        Expression::Select { selector, variants } => {
            for variant in variants {
                if is_arg(selector, arg)
                    && let VariantKey::NumberLiteral { value } = &variant.key
                    && let Ok(n) = value.parse::<f64>()
                    && !out.contains(&n)
                {
                    out.push(n);
                }
                exact_numbers(&variant.value, arg, out);
            }
        }
        Expression::Inline(InlineExpression::Placeable { expression }) => {
            exact_numbers_in(expression, arg, out)
        }
        Expression::Inline(_) => {}
    }
}

/// The plural category of `n` in `bundle`'s language, as Fluent itself has it: its own choice
/// of rules for the language (a region's, the language's, or English's).
fn category(bundle: &FluentBundle<FluentResource>, arg: &str, n: f64) -> &'static str {
    let variants = CATEGORIES
        .map(|c| Variant {
            key: VariantKey::Identifier { name: c },
            value: Pattern {
                elements: vec![PatternElement::TextElement { value: c }],
            },
            default: c == "other",
        })
        .to_vec();
    let pattern = Pattern {
        elements: vec![PatternElement::Placeable {
            expression: Expression::Select {
                selector: InlineExpression::VariableReference {
                    id: Identifier { name: arg },
                },
                variants,
            },
        }],
    };
    let mut args = FluentArgs::new();
    args.set(arg, n);
    let said = format_pattern(bundle, "a plural category", &pattern, Some(&args));
    CATEGORIES
        .into_iter()
        .find(|c| *c == said)
        .unwrap_or("other")
}

#[cfg(test)]
mod tests {
    use super::super::{Locale, Localizer};
    use super::PluralForms;

    /// `key`'s forms, by `n`, in a language `tag` whose one file is `ftl`.
    fn forms(tag: &'static str, ftl: &'static str, key: &str) -> PluralForms {
        let files: &'static [(&str, &str)] = Box::leak(Box::new([("a.ftl", ftl)]));
        let (localizer, errors) = Localizer::build(&[Locale { tag, files }], tag, tag);
        assert!(errors.is_empty(), "{errors:?}");
        localizer.tr_plurals(key, "n")
    }

    fn form<'f>(forms: &'f PluralForms, category: &str) -> &'f str {
        let found = forms.forms.iter().find(|(c, _)| *c == category);
        found.map_or("", |(_, text)| text)
    }

    #[test]
    fn each_category_takes_its_variant_and_the_count_s_place_is_kept() {
        let en = forms(
            "en",
            "count = { $n ->\n    [one] one thing\n   *[other] { $n } things\n}\n",
            "count",
        );
        assert_eq!(en.language, "en");
        assert_eq!(form(&en, "one"), "one thing");
        // Every category, the ones English doesn't use too: their default variant.
        for category in ["zero", "two", "few", "many", "other"] {
            assert_eq!(form(&en, category), "{n} things", "{category}");
        }
        assert!(en.exact.is_empty());
    }

    #[test]
    fn a_regional_language_keeps_its_forms() {
        // A bundle for en-GB: its `one` is the page's for 1 (Intl.PluralRules("en-GB")).
        let gb = forms(
            "en-GB",
            "count = { $n ->\n    [one] { $n } word\n   *[other] { $n } words\n}\n",
            "count",
        );
        assert_eq!(gb.language, "en-GB");
        assert_eq!(form(&gb, "one"), "{n} word");
        assert_eq!(form(&gb, "other"), "{n} words");
    }

    #[test]
    fn an_exact_number_s_variant_is_its_own_form_in_fluent_s_order() {
        let words = forms(
            "en",
            "count = { $n ->\n    [0] No words\n    [one] { $n } word\n   *[other] { $n } words\n}\n",
            "count",
        );
        // Zero isn't `other`'s: other counts are "2 words", never "No words".
        assert_eq!(form(&words, "other"), "{n} words");
        assert_eq!(form(&words, "one"), "{n} word");
        assert_eq!(words.exact, [("0".to_owned(), "No words".to_owned())]);
        // As Fluent goes, the first variant that fits: [one] before [1] takes 1, by the rules
        // of the language (a region's falling back to the language's).
        let ftl = "first = { $n ->\n    [one] A\n    [1] B\n   *[other] C\n}\n\
                   exactly = { $n ->\n    [1] B\n    [one] A\n   *[other] C\n}\n";
        assert_eq!(
            forms("en-GB", ftl, "first").exact,
            [("1".to_owned(), "A".to_owned())]
        );
        assert_eq!(
            forms("en-GB", ftl, "exactly").exact,
            [("1".to_owned(), "B".to_owned())]
        );
    }

    #[test]
    fn digits_in_the_text_stay_and_every_use_of_the_count_is_kept() {
        let section = forms("en", "count = Section 0: { $n } words ({ $n })\n", "count");
        for (category, text) in &section.forms {
            assert_eq!(text, "Section 0: {n} words ({n})", "{category}");
        }
        // A select on the count inside another's variant, and a nested placeable.
        let nested = forms(
            "en",
            "count = { $n ->\n    [one] { { $n } } item, 1 of { $n }\n   *[other] { $n } items\n}\n",
            "count",
        );
        assert_eq!(form(&nested, "one"), "{n} item, 1 of {n}");
    }

    #[test]
    fn polish_whole_numbers_and_fractions() {
        let pl = forms(
            "pl",
            "count = { $n ->\n    [one] { $n } A\n    [few] { $n } B\n    [many] { $n } C\n   \
             *[other] { $n } D\n}\n",
            "count",
        );
        let got: Vec<_> = ["one", "few", "many", "other"]
            .map(|c| form(&pl, c).to_owned())
            .to_vec();
        assert_eq!(got, ["{n} A", "{n} B", "{n} C", "{n} D"]);
    }
}

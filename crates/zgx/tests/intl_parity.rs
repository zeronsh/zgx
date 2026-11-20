//! `zgx::i18n::intl` against what Node's Intl gave (`fixtures/intl_capture.mjs`),
//! for en, de, ja and fi in each function. Run with `--features i18n-intl`.
use serde_json::Value;
use zgx::i18n::intl::*;

fn style(v: Option<&Value>) -> Style {
    match v.and_then(Value::as_str) {
        Some("short") => Style::Short,
        Some("narrow") => Style::Narrow,
        _ => Style::Long,
    }
}
fn width(v: Option<&Value>) -> Option<Width> {
    v.and_then(Value::as_str).map(|s| match s {
        "long" => Width::Long,
        "short" => Width::Short,
        "narrow" => Width::Narrow,
        _ => Width::Numeric,
    })
}
fn date_style(v: Option<&Value>) -> Option<DateStyle> {
    v.and_then(Value::as_str).map(|s| match s {
        "full" => DateStyle::Full,
        "long" => DateStyle::Long,
        "medium" => DateStyle::Medium,
        _ => DateStyle::Short,
    })
}

fn run(case: &Value) -> String {
    let locale = case["locale"].as_str().unwrap();
    let args = &case["args"];
    let o = &args["options"];
    match case["fn"].as_str().unwrap() {
        "list" => {
            let items: Vec<String> = args["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            let kind = match o["type"].as_str() {
                Some("disjunction") => ListType::Disjunction,
                Some("unit") => ListType::Unit,
                _ => ListType::Conjunction,
            };
            format_list(locale, &items, kind, style(o.get("style")))
        }
        "number" => {
            let options = NumberOptions {
                style: match o["style"].as_str() {
                    Some("percent") => NumberStyle::Percent,
                    Some("currency") => NumberStyle::Currency,
                    _ => NumberStyle::Decimal,
                },
                currency: o["currency"].as_str().map(String::from),
                minimum_fraction_digits: o["minimumFractionDigits"].as_u64().map(|n| n as u8),
                maximum_fraction_digits: o["maximumFractionDigits"].as_u64().map(|n| n as u8),
                use_grouping: o["useGrouping"].as_bool(),
            };
            format_number(locale, args["n"].as_f64().unwrap(), &options)
        }
        "date" => {
            let options = DateOptions {
                date_style: date_style(o.get("dateStyle")),
                time_style: date_style(o.get("timeStyle")),
                weekday: width(o.get("weekday")),
                year: width(o.get("year")),
                month: width(o.get("month")),
                day: width(o.get("day")),
                ..Default::default()
            };
            format_date(locale, args["t"].as_f64().unwrap(), Some(0), &options)
        }
        "relative" => format_relative(
            locale,
            args["delta"].as_f64().unwrap(),
            Unit::parse(args["unit"].as_str().unwrap()).unwrap(),
            o["numeric"] == "auto",
            style(o.get("style")),
        ),
        other => panic!("{other}"),
    }
}

/// Spaces Chrome writes as no-break (U+202F, U+00A0) compare as they are; a
/// mismatch lists the case.
#[test]
fn intl_output_matches_nodes() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/intl_node.json")).unwrap();
    let mut misses = Vec::new();
    let cases = fixture["cases"].as_array().unwrap();
    for case in cases {
        let got = run(case);
        let want = case["expected"].as_str().unwrap();
        if got != want {
            misses.push(format!(
                "{} {} {}: got {got:?}, Node {want:?}",
                case["fn"], case["locale"], case["args"]["options"]
            ));
        }
    }
    assert!(
        misses.is_empty(),
        "{} of {} differ:\n{}",
        misses.len(),
        cases.len(),
        misses.join("\n")
    );
}

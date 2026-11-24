//! A native module written with `zgx_module::module`, for zgx-script's
//! tests (`tests/native_modules.rs`): one function for each kind of
//! argument and result, its manifest, and what a script is told when it
//! passes the wrong thing.

/// What the module keeps for `keep` and `kept`: one for each thread.
#[derive(Default)]
pub struct Kept {
    value: zgx_module::Value,
    times: u32,
}

#[zgx_module::module(name = "typed")]
pub mod typed {
    use zgx_module::{Failed, List, Map, Promise, Record, Ref, Text, Value, export};

    use super::Kept;

    /// What `find` is asked: an id, and how many tags at most.
    #[derive(Record)]
    pub struct Query<'a> {
        pub id: &'a str,
        pub limit: Option<u32>,
    }

    #[derive(Record)]
    pub struct Found {
        pub id: String,
        pub n: f64,
        pub tags: Vec<String>,
    }

    /// A row as `rows` reads it: its text borrowed, where the runtime holds
    /// it.
    #[derive(Record)]
    pub struct Row<'a> {
        pub id: &'a str,
        pub text: &'a str,
        pub n: f64,
        pub done: bool,
    }

    /// The record `query` asks for, found later on a thread of the
    /// module's: rejected if its id is empty.
    #[export(async)]
    pub fn find_later(promise: Promise<Found>, query: Query) {
        let id = query.id.to_owned();
        std::thread::spawn(move || {
            promise.resolve_with(move |_| match id.is_empty() {
                true => Err(Failed("has no id".into())),
                false => Ok(Found {
                    id: id.to_uppercase(),
                    n: id.len() as f64,
                    tags: Vec::new(),
                }),
            });
        });
    }

    /// How many words `text` has.
    #[export]
    pub fn count_words(text: &str) -> f64 {
        text.split_whitespace().count() as f64
    }

    /// `a` and `b` with `sep` between them, a space if none is given.
    #[export]
    pub fn join(a: &str, b: &str, sep: Option<&str>) -> String {
        format!("{a}{}{b}", sep.unwrap_or(" "))
    }

    /// The record `query` asks for; nil if its id is empty.
    #[export]
    pub fn find(query: Query) -> Option<Found> {
        let limit = query.limit.unwrap_or(2) as usize;
        (!query.id.is_empty()).then(|| Found {
            id: query.id.to_uppercase(),
            n: query.id.len() as f64,
            tags: ["one", "two", "three"]
                .iter()
                .take(limit)
                .map(|t| t.to_string())
                .collect(),
        })
    }

    #[export]
    pub fn sum(numbers: List<f64>) -> f64 {
        numbers.iter().sum()
    }

    /// The prices' total, and the dearest's name.
    #[export]
    pub fn dearest(prices: Map<f64>) -> Option<String> {
        prices
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name.to_owned())
    }

    #[export]
    pub fn nothing() {}

    /// Registered under a name of its own.
    #[export(name = "other.doubled")]
    pub fn doubled(n: i32) -> i32 {
        n * 2
    }

    /// Fails for 13, as a function that can't answer does.
    #[export]
    pub fn checked(n: u8) -> Result<u8, Failed> {
        if n == 13 {
            return Err(Failed::from("is unlucky"));
        }
        Ok(n)
    }

    /// What it was given.
    #[export]
    pub fn any(value: Ref) -> Value {
        value.to_owned()
    }

    /// A string of any length, and how long it is.
    #[export]
    pub fn long(text: Text) -> usize {
        text.len()
    }

    /// Keeps `value` for `kept`.
    #[export(action)]
    pub fn keep(#[state] kept: &mut Kept, value: Ref) {
        // (What it held is freed after this returns the state.)
        kept.value = value.to_owned();
        kept.times += 1;
    }

    /// What `keep` was last given, and how many times it was called.
    #[export]
    pub fn kept(#[state] kept: &Kept) -> Vec<Value> {
        vec![kept.value.clone(), Value::from(f64::from(kept.times))]
    }

    /// An action that fails unless it is given a string.
    #[export(action)]
    pub fn must(text: &str) -> Result<(), Failed> {
        if text.is_empty() {
            return Err(Failed::from("was given nothing to do"));
        }
        Ok(())
    }

    /// A string's length: an argument read and a number made
    /// (`module_call_bench`'s, beside the fixture's by hand).
    #[export]
    pub fn length(text: &str) -> f64 {
        text.len() as f64
    }

    /// Rows read as records (`module_call_bench`'s `rows`).
    #[export]
    pub fn rows(rows: List<Row>) -> f64 {
        let mut sum = 0.;
        for row in rows.iter() {
            sum += row.n + (row.id.len() + row.text.len()) as f64 + f64::from(u8::from(row.done));
        }
        sum
    }
}

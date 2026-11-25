//! A script's or an app's shader, checked as the renderer will compile it.
//!
//! What is checked is the user's source in the module the renderer builds
//! around it: the quad shader, whose every line is parsed and validated for
//! each shader, though only the user's can be wrong. So first a smaller module
//! is asked: the same, with the quad shader's functions left their names,
//! arguments and result, and nothing in them ([`Part`]). It may only say yes,
//! and only of a source it can answer for ([`plain`]); whatever it doesn't is
//! checked whole, whose word, and whose error, is the answer.
//!
//! A yes that was wrong would show when the shader is first drawn and no
//! sooner: whatever draws it (wgpu, Metal, Direct3D 11) validates the whole
//! module again, off the frame, before it builds anything of it
//! (`fill_validates`). One that isn't valid never reaches the driver: its fill
//! draws plain, its layer as painted, and the script isn't told.
use std::{collections::HashSet, sync::OnceLock};
use wgpu::naga;

/// One of the renderer's own texts (draw.wgsl, blend.wgsl), as the smaller
/// module has it.
#[derive(Default)]
pub(crate) struct Part {
    /// Its structs and variables as they are, its functions emptied, its
    /// entry points and comments left out.
    pub(crate) text: String,
    /// What its functions and entry points are called.
    pub(crate) functions: HashSet<String>,
}
impl Part {
    fn of(source: &str) -> Self {
        let source = without_comments(source).unwrap_or_default();
        let mut part = Self::default();
        let mut item = String::new();
        let (mut braces, mut others) = (0usize, 0usize);
        for c in source.chars() {
            item.push(c);
            match c {
                '{' => braces += 1,
                '}' => braces -= 1,
                '(' | '[' => others += 1,
                ')' | ']' => others -= 1,
                _ => {}
            }
            if braces == 0 && others == 0 && matches!(c, '}' | ';') {
                part.take(item.trim());
                item.clear();
            }
        }
        part.take(item.trim());
        part
    }

    /// An item of the text: a function, or a declaration kept as it is.
    fn take(&mut self, item: &str) {
        let head = item.split('{').next().unwrap_or_default();
        let Some(at) = words(head)
            .find(|(_, word)| *word == "fn")
            .map(|(at, _)| at)
        else {
            self.text.push_str(item);
            self.text.push('\n');
            return;
        };
        let name = words(&head[at + 2..])
            .next()
            .map_or(String::new(), |(_, word)| word.to_owned());
        self.functions.insert(name);
        if words(&head[..at]).next().is_some() {
            // `@vertex`, `@fragment`: nothing calls one.
            return;
        }
        self.text.push_str(head);
        self.text.push_str(&match head.rsplit_once("->") {
            Some((_, result)) => format!("{{ var zero:{}; return zero; }}\n", result.trim()),
            None => "{}\n".to_owned(),
        });
    }
}

/// draw.wgsl and blend.wgsl, so.
pub(crate) fn parts() -> &'static [Part; 2] {
    static PARTS: OnceLock<[Part; 2]> = OnceLock::new();
    PARTS.get_or_init(|| {
        [
            Part::of(include_str!("draw.wgsl")),
            Part::of(include_str!("blend.wgsl")),
        ]
    })
}

/// `source` with each comment a space. None where one doesn't end (a nested
/// one's counts), which would hide what comes after.
fn without_comments(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                // (WGSL's line ends, all of them.)
                for c in chars.by_ref() {
                    if matches!(c, '\u{a}'..='\u{d}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut depth = 1;
                while depth > 0 {
                    match (chars.next()?, chars.peek()) {
                        ('*', Some('/')) => {
                            chars.next();
                            depth -= 1;
                        }
                        ('/', Some('*')) => {
                            chars.next();
                            depth += 1;
                        }
                        _ => {}
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    Some(out)
}

/// The words of `text`, and where each is.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let part = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut at = 0;
    std::iter::from_fn(move || {
        let start = at + text[at..].find(part)?;
        let len = text[start..]
            .find(|c| !part(c))
            .unwrap_or(text.len() - start);
        at = start + len;
        Some((start, &text[start..at]))
    })
}

/// Whether the smaller module can answer for `user`: what it declares and
/// uses is the same there as in the whole. Not where it names one of the
/// renderer's functions (emptied, or left out); declares a binding, a module
/// variable or an entry point of its own, which share numbers and names with
/// the renderer's; discards, which the renderer's functions called after it
/// must allow; or has a directive, a letter past ASCII's, or a comment that
/// doesn't end.
fn plain(user: &str, parts: &[&Part]) -> bool {
    let Some(text) = without_comments(user) else {
        return false;
    };
    if !text.is_ascii() || text.contains('@') {
        return false;
    }
    words(&text).all(|(at, word)| {
        let after = text[at + word.len()..].trim_start();
        !(word == "var" && after.starts_with('<'))
            && !matches!(
                word,
                "override" | "enable" | "requires" | "diagnostic" | "discard"
            )
            && !parts.iter().any(|part| part.functions.contains(word))
    })
}

/// Whether `user` is known to be right by the smaller module `module` builds
/// of `parts`, in place of the renderer's texts.
fn known(user: &str, parts: &[&Part], module: impl FnOnce() -> String) -> bool {
    plain(user, parts)
        && naga::front::wgsl::parse_str(&module()).is_ok_and(|module| valid(&module).is_ok())
}

/// `module`, validated as every module of a shader's is.
fn valid(module: &naga::Module) -> Result<(), Box<naga::WithSpan<naga::valid::ValidationError>>> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(module)
    .map(|_| ())
    .map_err(Box::new)
}

/// Checks a layer shader's source (the user's part) as the renderer will
/// compile it, as [`check_fill_shader`] does a fill's.
pub fn check_layer_shader(user: &str) -> Result<(), String> {
    let [draw, blend] = parts();
    if known(user, &[draw, blend], || {
        crate::filter_source_of(&draw.text, &blend.text, user)
    }) {
        return Ok(());
    }
    check_layer_shader_whole(user)
}
/// Checks a shader fill's source (the user's part, as `FillShader` holds
/// it) as the renderer will compile it: `Err` reads like a compiler
/// message, with lines counted in `user`, for an app or script to show.
pub fn check_fill_shader(user: &str) -> Result<(), String> {
    let [draw, _] = parts();
    if known(user, &[draw], || crate::fill_source_of(&draw.text, user)) {
        return Ok(());
    }
    check_fill_shader_whole(user)
}
/// [`check_layer_shader`], the whole module always. For tests.
#[doc(hidden)]
pub fn check_layer_shader_whole(user: &str) -> Result<(), String> {
    check_module(user, crate::filter_source)
}
/// [`check_fill_shader`], the whole module always. For tests.
#[doc(hidden)]
pub fn check_fill_shader_whole(user: &str) -> Result<(), String> {
    check_module(user, crate::fill_source)
}
/// Whether the smaller module answered for a fill's source (else a layer
/// shader's). For tests.
#[doc(hidden)]
pub fn check_is_known(user: &str, fill: bool) -> bool {
    let [draw, blend] = parts();
    if fill {
        known(user, &[draw], || crate::fill_source_of(&draw.text, user))
    } else {
        known(user, &[draw, blend], || {
            crate::filter_source_of(&draw.text, &blend.text, user)
        })
    }
}

/// Checks `user` in the module `module` builds around it.
fn check_module(user: &str, module: fn(&str) -> String) -> Result<(), String> {
    let source = module(user);
    // Lines before the user's code in the module `module` builds.
    let template = module("\u{1}");
    let before = template[..template.find('\u{1}').unwrap_or(0)]
        .matches('\n')
        .count();
    let place = |location: Option<naga::SourceLocation>| match location {
        Some(at) if at.line_number as usize > before => {
            format!("line {}: ", at.line_number as usize - before)
        }
        Some(_) => "in zgx's part of the module (a reserved name?): ".to_owned(),
        None => String::new(),
    };
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|e| format!("{}{}", place(e.location(&source)), e.message()))?;
    valid(&module).map_err(|e| format!("{}{}", place(e.location(&source)), e.as_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smaller module's parts: no function's inside, no entry point, the
    /// rest there; and it is right by itself.
    #[test]
    fn the_parts_keep_what_a_shader_sees_and_nothing_else() {
        let [draw, blend] = parts();
        for kept in [
            "struct Region",
            "struct Out",
            "var atlas",
            "var tex_sampler",
            "var<uniform> viewport",
        ] {
            assert!(draw.text.contains(kept), "{kept}");
        }
        assert!(blend.text.contains("var zgx_behind"));
        for function in [
            "shade",
            "shade_texel",
            "rounded",
            "region",
            "vs",
            "fs",
            "basic",
            "fs_both",
        ] {
            assert!(draw.functions.contains(function), "{function}");
        }
        for function in ["zgx_blend", "zgx_beneath", "zgx_lum"] {
            assert!(blend.functions.contains(function), "{function}");
        }
        for (part, whole) in [
            (draw, include_str!("draw.wgsl")),
            (blend, include_str!("blend.wgsl")),
        ] {
            assert!(!part.text.contains("@fragment") && !part.text.contains("//"));
            // Every function of the text, by a search of its own.
            let found = without_comments(whole).unwrap().matches("fn ").count();
            assert_eq!(part.functions.len(), found);
            assert!(
                part.text.len() * 2 < whole.len(),
                "{} of {}",
                part.text.len(),
                whole.len()
            );
        }
        assert!(check_is_known(
            "fn fill(i:FillInput)->vec4<f32> { return vec4(1.); }",
            true
        ));
        assert!(check_is_known(
            "fn layer(i:LayerInput)->vec4<f32> { return i.color; }",
            false
        ));
    }

    /// What the smaller module doesn't answer for, judged without comments.
    #[test]
    fn only_a_plain_source_is_answered_for() {
        let fill = |more: &str| {
            format!("{more}\nfn fill(i:FillInput)->vec4<f32> {{ return vec4(i.p.x); }}")
        };
        let known = |more: &str| check_is_known(&fill(more), true);
        for yes in [
            "",
            "// @group(0) discard /* not open\n",
            "/* var<private> a /* nested */ override */",
            "const K=2.; fn mine(a:f32)->f32 { return a*K; }",
            "fn used(p:vec2<f32>)->f32 { var a=1.; if a<p.x { a=2.; } return a; }",
            "fn reads(p:vec2<f32>)->vec4<f32> { return textureSampleLevel(atlas,tex_sampler,p,0.); }",
        ] {
            assert!(known(yes), "{yes}");
        }
        for no in [
            "@group(3) @binding(0) var<uniform> q:vec4<f32>;",
            "@fragment fn other()->@location(0) vec4<f32> { return vec4(1.); }",
            "var<private> q:f32;",
            "var <private> q:f32;",
            "var/* x */<private> q:f32;",
            "override q:f32=1.;",
            "fn d() { discard; }",
            "fn uses(v:Out)->vec4<f32> { return shade(v); }",
            "fn rounded()->f32 { return 1.; }",
            "fn f(region:f32)->f32 { return region; }",
            "const \u{3b1}=1.;",
            "/* never ends",
            "/* never /* ends */",
            "fn broken( { }",
        ] {
            assert!(!known(no), "{no}");
        }
    }

    /// Whether the smaller module alone, with no source kept from it, would
    /// say yes.
    fn careless(user: &str, fill: bool) -> bool {
        let [draw, blend] = parts();
        let module = match fill {
            true => crate::fill_source_of(&draw.text, user),
            false => crate::filter_source_of(&draw.text, &blend.text, user),
        };
        naga::front::wgsl::parse_str(&module).is_ok_and(|module| valid(&module).is_ok())
    }

    /// Sources the whole module refuses and the smaller alone would take:
    /// each is why `plain` keeps its kind from it.
    #[test]
    fn what_the_smaller_module_would_wrongly_take_is_checked_whole() {
        let fill = |before: &str, body: &str| {
            format!("{before}\nfn fill(i:FillInput)->vec4<f32> {{ {body} return vec4(i.p.x); }}")
        };
        let mut wrong = 0;
        let mut seen = Vec::new();
        for (why, user) in [
            // Names the smaller module leaves out: the renderer's entry points.
            (
                "an entry point's name",
                fill("fn vs()->f32 { return 1.; }", ""),
            ),
            ("an entry point's name", fill("const fs=1.;", "")),
            (
                "an entry point's name",
                fill("struct fs_both { a:f32 }", ""),
            ),
            // A binding the renderer's functions use and these don't.
            (
                "a binding of the renderer's",
                fill(
                    "@group(0) @binding(1) var mine:texture_2d<f32>;",
                    "let t=textureLoad(mine,vec2(0),0); if t.x>2. { return t; }",
                ),
            ),
            (
                "a binding of the renderer's",
                fill(
                    "@group(0) @binding(2) var mine:sampler;",
                    "let t=textureSampleLevel(atlas,mine,i.p,0.); if t.x>2. { return t; }",
                ),
            ),
            (
                "a binding of the renderer's",
                fill(
                    "@group(1) @binding(1) var<storage, read> mine:array<vec4<f32>>;",
                    "if mine[0].x>2. { return mine[0]; }",
                ),
            ),
        ] {
            let whole = check_fill_shader_whole(&user);
            assert!(whole.is_err(), "{why}: {user}");
            assert_eq!(check_fill_shader(&user), whole, "{why}");
            assert!(!check_is_known(&user, true), "{why}");
            if careless(&user, true) {
                wrong += 1;
            } else {
                seen.push(user);
            }
        }
        assert!(wrong >= 3, "{wrong} would be taken; not: {seen:?}");
    }
}

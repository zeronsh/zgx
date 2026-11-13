//! Static styles as shared classes: each distinct look (an element's static
//! styles with its hover, pressed, focus and disabled ones) becomes one
//! rule, made once and given to every element that has it, so a thousand
//! rows styled alike carry one class name, not a thousand inline styles.
//! Looks are compared as styles, so a look seen before costs no CSS text;
//! its class name crosses to JS once, then by index.

use crate::ops::{self, Ops};
use std::{cell::RefCell, collections::HashMap};
use zgx::compose::__dom::Look;

/// What every element starts from: zgx's layout defaults, which are mostly
/// CSS flexbox's. `.z` is a box, `.zt` text; `.zb` a button, `.ze` an
/// editor, their browser looks reset; `.zc` lays out as its child
/// (`display: contents`); `.zr` holds absolute children.
pub(crate) const BASE_CSS: &str = "\
.z{display:flex;flex-direction:column;box-sizing:border-box;}\
.zr{position:relative;}\
.zv{position:absolute;left:0;right:0;}\
.zb{font:inherit;color:inherit;background:none;text-align:inherit;cursor:default;margin:0;padding:0;border:0 solid transparent;}\
.zb:focus-visible,.ze:focus-visible{outline:2px solid Highlight;outline-offset:-2px;}\
.ze{font:inherit;color:inherit;background:transparent;outline:none;white-space:pre-wrap;margin:0;padding:0;border:0 solid transparent;}\
.zc{display:contents;}\
.zt{display:block;box-sizing:border-box;}\
.zs0{user-select:none;}.zs1{user-select:text;}\
.zu{align-items:center;justify-content:center;border:1px dashed currentColor;opacity:.6;min-height:24px;}\
.zd,.zp{color:inherit;background:none;border:0 solid transparent;padding:0;max-width:calc(100vw - 8px);max-height:calc(100vh - 8px);}\
.zd:not([open]),.zp:not(:popover-open){display:none;}\
.zd::backdrop{background:var(--zbd,rgba(0,0,0,.5));}\
.zp{position:fixed;inset:auto;margin:0;overflow:visible;}\
.zp[data-layer]{inset:0;width:auto;height:auto;max-width:none;max-height:none;pointer-events:none;}\
.zp[data-layer]>*{pointer-events:auto;}\
.zp[role=tooltip]{transition:opacity .14s ease-out;}\
.za{overflow:visible;}\
.za::before{content:\"\";position:absolute;z-index:-1;left:var(--zx);top:var(--zy);width:var(--zs);height:var(--zs);background:inherit;transform:translate(-50%,-50%) rotate(45deg);}\
@starting-style{.zp[role=tooltip]:popover-open{opacity:0;}}\
";

/// `base`'s own declarations, folded into a look's rule so its elements
/// match one class, and the classes it keeps beside that rule's.
fn folded(base: &'static str) -> (&'static str, &'static str) {
    match base {
        "z" => (Z, ""),
        "zt" => (ZT, ""),
        "z zb" => (Z, "zb "),
        "z ze" => (Z, "ze "),
        "z zu" => (Z, "zu "),
        "z zc" => (Z, "zc "),
        "z zd" => (Z, "zd "),
        "z zp" => (Z, "zp "),
        _ => ("", ""),
    }
}
const Z: &str = "display:flex;flex-direction:column;box-sizing:border-box;";
const ZT: &str = "display:block;box-sizing:border-box;";

/// How many looks are compared before the CSS text is: a view's rows
/// repeat a few.
const RECENT: usize = 64;

#[derive(Default)]
struct Sheet {
    /// Looks seen lately and their class names' indices, newest last.
    recent: Vec<(&'static str, Look, u32)>,
    /// Class names by their base classes and CSS, for looks unseen lately.
    by_css: HashMap<(&'static str, [Option<String>; 5]), u32>,
    /// Plain class names (`z zc`) by index.
    plain: HashMap<&'static str, u32>,
    /// Rules made, for class numbers.
    rules: u32,
    names: u32,
}

thread_local! {
    static SHEET: RefCell<Sheet> = RefCell::default();
}

impl Sheet {
    fn name(&mut self, ops: &mut Ops, name: &str) -> u32 {
        let index = self.names;
        self.names += 1;
        ops.define(&[ops::CLASS_NAME, index], name);
        index
    }
}

/// The index of the class name for base classes `base` (`z`, `zt zb`…)
/// alone.
pub(crate) fn plain(ops: &mut Ops, base: &'static str) -> u32 {
    SHEET.with(|sheet| {
        let mut sheet = sheet.borrow_mut();
        if let Some(index) = sheet.plain.get(base) {
            return *index;
        }
        let index = sheet.name(ops, base);
        sheet.plain.insert(base, index);
        index
    })
}

/// The index of the class name for `base` and `look`, its rules written the
/// first time.
pub(crate) fn class(ops: &mut Ops, base: &'static str, look: Look) -> u32 {
    if look.is_empty() {
        return plain(ops, base);
    }
    SHEET.with(|sheet| {
        let mut sheet = sheet.borrow_mut();
        if let Some(at) = sheet
            .recent
            .iter()
            .rposition(|(b, l, _)| *b == base && *l == look)
        {
            return sheet.recent[at].2;
        }
        let [hover, active, focus, disabled] = look.variants_css();
        let key = (base, [Some(look.css()), hover, active, focus, disabled]);
        let index = match sheet.by_css.get(&key) {
            Some(index) => *index,
            None => {
                let class = sheet.rules;
                sheet.rules += 1;
                let mut rule = |selector: String, css: &str| {
                    if !css.is_empty() {
                        ops.define(&[ops::RULE], &format!("{selector}{{{css}}}"));
                    }
                };
                let [own, hover, active, focus, disabled] = &key.1;
                let (base_css, kept) = folded(base);
                rule(
                    format!(".c{class}"),
                    &format!("{base_css}{}", own.as_deref().unwrap_or("")),
                );
                if let Some(css) = hover {
                    rule(format!(".c{class}:hover"), css);
                }
                if let Some(css) = focus {
                    rule(format!(".c{class}:focus-visible"), css);
                }
                if let Some(css) = active {
                    rule(format!(".c{class}:active"), css);
                }
                if let Some(css) = disabled {
                    rule(
                        format!(".c{class}:disabled,.c{class}[aria-disabled='true']"),
                        css,
                    );
                }
                let index = match base_css {
                    "" => sheet.name(ops, &format!("{base} c{class}")),
                    _ => sheet.name(ops, &format!("{kept}c{class}")),
                };
                sheet.by_css.insert(key, index);
                index
            }
        };
        if sheet.recent.len() == RECENT {
            sheet.recent.remove(0);
        }
        sheet.recent.push((base, look, index));
        index
    })
}

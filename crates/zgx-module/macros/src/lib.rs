//! zgx-module's macros, used as `zgx_module::{module, export, Record}`.
//!
//! `#[module]` on a `mod` makes it a native module: each function in it
//! marked `#[export]` is registered for scripts to call, its arguments read
//! from the values a script passed by their Rust types and its result made
//! into one, and the module's manifest (its name, its functions and their
//! types, as JSON) is written into the library for the runtime and the
//! editor's types to read. `#[derive(Record)]` makes a struct an argument or
//! a result: a map of its fields.
//!
//! The types are read as written: a manifest is text in the library, made
//! here, before the compiler knows what a name means. So an argument's type
//! is one of those docs/modules.md lists, by its own name, or a record
//! declared in the module.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote};
use syn::{
    Attribute, Error, Fields, FnArg, GenericArgument, GenericParam, Item, ItemFn, ItemMod,
    ItemStruct, LitStr, Pat, PathArguments, Result, ReturnType, Type, parse_macro_input,
    spanned::Spanned,
};

/// What `#[module]` leaves in a function's `#[export]` once it has read it.
const IN_MODULE: &str = "__in_module";

/// The interface these macros write for (`zgx_module::abi::ABI`): what
/// they write fails to build against another.
const ABI: u32 = 1;

/// Makes a `mod` a native module. `name` is what scripts ask for it by
/// (`ui.module("words")`) and what its functions' names begin with
/// (`words.count`).
///
/// ```ignore
/// #[zgx_module::module(name = "words")]
/// mod words {
///     #[zgx_module::export]
///     fn count(text: &str) -> f64 {
///         text.split_whitespace().count() as f64
///     }
/// }
/// ```
///
/// One to a library: it writes the library's entry and its manifest.
#[proc_macro_attribute]
pub fn module(args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemMod);
    match module_of(args.into(), item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Marks a function of a `#[module]` for scripts to call: as
/// `<module>.<function>` with `ui.call_sync`, and as a member of the
/// module's table (`ui.module`).
///
/// - `#[export(name = "chat.reply_run")]` registers it under that name.
/// - `#[export(action)]` registers it as an action (`ui.action`,
///   `ui.invoke`): one argument, nothing returned.
/// - `#[export(async)]` registers it as an async action (`ui.call`, a
///   promise): its first parameter (after the state) is the
///   `Promise<T>` it settles later, then one argument at most, nothing
///   returned. Its member on the module's table gives the promise.
/// - A first parameter marked `#[state]`, `&T` or `&mut T`, is the module's
///   state of that type for the thread (`Host::state`).
/// - Returning `Result<T, E>` raises an `Err` in the script that called.
#[proc_macro_attribute]
pub fn export(args: TokenStream, item: TokenStream) -> TokenStream {
    // (Inside a `#[module]`, which read it and did the work: it stays on
    // the function, marked, so its import is used.)
    if args.to_string().starts_with(IN_MODULE) {
        return item;
    }
    let item = parse_macro_input!(item as ItemFn);
    Error::new(
        item.sig.ident.span(),
        "#[export] marks a function inside a `#[zgx_module::module] mod`: that is what registers it",
    )
    .to_compile_error()
    .into()
}

/// Makes a struct an argument or a result of a module's functions: a map
/// of its fields, by their names. A field borrowed from the argument
/// (`&'a str`) is read where the runtime holds it.
#[proc_macro_derive(Record)]
pub fn record(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemStruct);
    match record_of(&item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// A JSON string of `text`.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The last name of a type's path and its type arguments, if it is a path.
fn named(ty: &Type) -> Option<(String, Vec<&Type>)> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    let args = match &last.arguments {
        PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(ty) => Some(ty),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some((last.ident.to_string(), args))
}

/// A Rust type as the manifest says it (docs/modules.md), read as written.
/// `records` are the module's; `returned` allows what only a result is.
fn manifest_type(ty: &Type, records: &[String], returned: bool) -> Result<String> {
    let unknown = |what: &str| {
        Err(Error::new(
            ty.span(),
            format!(
                "{what} isn't a type a script can pass: use bool, a number type, &str, String, \
                 Option, Vec, zgx_module's List, Map, Ref or Value, or a struct of this module \
                 that derives Record"
            ),
        ))
    };
    let of = |ty: &Type| manifest_type(ty, records, returned);
    match ty {
        Type::Reference(reference) => match &*reference.elem {
            Type::Path(path) if path.path.is_ident("str") => Ok("\"string\"".into()),
            Type::Slice(slice) if returned => Ok(format!("{{\"list\":{}}}", of(&slice.elem)?)),
            _ => unknown("this reference"),
        },
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok("\"nil\"".into()),
        Type::Paren(paren) => of(&paren.elem),
        Type::Group(group) => of(&group.elem),
        Type::Path(_) => {
            let Some((name, args)) = named(ty) else {
                return unknown("this type");
            };
            let one = |what: &str| match args.as_slice() {
                [of] => manifest_type(of, records, returned),
                _ => Err(Error::new(ty.span(), format!("{what} of what?"))),
            };
            match name.as_str() {
                "bool" => Ok("\"boolean\"".into()),
                "f64" | "f32" | "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32"
                | "u64" | "usize" => Ok("\"number\"".into()),
                "String" | "Text" => Ok("\"string\"".into()),
                "Ref" | "Value" => Ok("\"any\"".into()),
                "Option" => Ok(format!("{{\"optional\":{}}}", one("Option")?)),
                "Vec" | "List" => Ok(format!("{{\"list\":{}}}", one(&name)?)),
                "Map" => Ok(format!("{{\"map\":{}}}", one("Map")?)),
                "HashMap" | "BTreeMap" => match args.as_slice() {
                    [key, value] if matches!(named(key), Some((k, _)) if k == "String") => {
                        Ok(format!("{{\"map\":{}}}", of(value)?))
                    }
                    _ => Err(Error::new(
                        ty.span(),
                        "a map a script passes has String keys",
                    )),
                },
                _ if records.contains(&name) => {
                    Ok(format!("{{\"record\":{}}}", json_string(&name)))
                }
                _ => unknown(&format!("`{name}`")),
            }
        }
        _ => unknown("this type"),
    }
}

/// An item's doc comment as one text, or nothing.
fn docs(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(meta) = &attr.meta
            && let syn::Expr::Lit(lit) = &meta.value
            && let syn::Lit::Str(text) = &lit.lit
        {
            lines.push(text.value().trim().to_owned());
        }
    }
    lines.join("\n").trim().to_owned()
}

/// Whether `attr` is `name`, written bare or by a path ending in it
/// (`zgx_module::export`).
fn is_attr(attr: &Attribute, name: &str) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|last| last.ident == name)
}
/// Whether a struct derives `Record` (by that name, or a path ending in
/// it).
fn derives_record(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let mut found = false;
        if attr.path().is_ident("derive") {
            let _ = attr.parse_nested_meta(|meta| {
                found |= meta
                    .path
                    .segments
                    .last()
                    .is_some_and(|last| last.ident == "Record");
                Ok(())
            });
        }
        found
    })
}

/// What `#[export(…)]` says.
#[derive(Default)]
struct Export {
    name: Option<String>,
    action: bool,
    /// An async action: its first parameter (after the state) a
    /// `Promise<T>` it settles later.
    asynchronous: bool,
}
fn export_of(attr: &Attribute) -> Result<Export> {
    let mut export = Export::default();
    if matches!(attr.meta, syn::Meta::Path(_)) {
        return Ok(export);
    }
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("name") {
            export.name = Some(meta.value()?.parse::<LitStr>()?.value());
            Ok(())
        } else if meta.path.is_ident("action") {
            export.action = true;
            Ok(())
        } else if meta.path.is_ident("async") {
            export.asynchronous = true;
            Ok(())
        } else {
            Err(meta.error("#[export] takes `name = \"…\"`, `action` and `async`"))
        }
    })?;
    Ok(export)
}

/// One exported function: what registers it, and its place in the manifest.
struct Exported {
    wrapper: Tokens,
    register: Tokens,
    /// `"name":{…}`.
    manifest: String,
    action: bool,
    asynchronous: bool,
}

fn exported(
    function: &mut ItemFn,
    export: Export,
    module: &str,
    records: &[String],
) -> Result<Exported> {
    let ident = function.sig.ident.clone();
    let no = |span: Span, what: &str| Err(Error::new(span, what));
    if function.sig.asyncness.is_some() {
        return no(
            function.sig.span(),
            "an exported function runs on the interface's thread, to its end: it can't be async",
        );
    }
    if function.sig.generics.type_params().next().is_some()
        || function.sig.generics.const_params().next().is_some()
    {
        return no(
            function.sig.generics.span(),
            "an exported function can't be generic over types",
        );
    }
    let name = export.name.unwrap_or_else(|| format!("{module}.{ident}"));
    let wrapper = format_ident!("__zgx_call_{}", ident);
    let mut state = None;
    let mut state_mut = false;
    let mut reads = Vec::new();
    let mut passed = Vec::new();
    let mut args_json = Vec::new();
    let mut index = 0usize;
    // An async action's promise, and what it resolves to.
    let mut resolves = None;
    for (at, arg) in function.sig.inputs.iter_mut().enumerate() {
        let FnArg::Typed(arg) = arg else {
            return no(arg.span(), "an exported function takes no `self`");
        };
        let marked = arg.attrs.iter().position(|attr| is_attr(attr, "state"));
        if let Some(marked) = marked {
            arg.attrs.remove(marked);
            if at != 0 {
                return no(arg.span(), "#[state] marks the first parameter");
            }
            let Type::Reference(reference) = &*arg.ty else {
                return no(
                    arg.ty.span(),
                    "the module's state is passed as `&T` or `&mut T`",
                );
            };
            let of = &reference.elem;
            state = Some(quote!(#of));
            state_mut = reference.mutability.is_some();
            passed.push(if reference.mutability.is_some() {
                quote!(&mut *__state)
            } else {
                quote!(&*__state)
            });
            continue;
        }
        if export.asynchronous && resolves.is_none() {
            let Some((name, of)) =
                named(&arg.ty).filter(|(name, of)| name == "Promise" && of.len() == 1)
            else {
                return no(
                    arg.ty.span(),
                    "an async action's first parameter (after its state) is the `Promise<T>` it settles",
                );
            };
            let _ = name;
            let of = of[0];
            resolves = Some(manifest_type(of, records, true)?);
            passed.push(quote!(::zgx_module::__private::promise::<#of>(__promise)));
            continue;
        }
        let Pat::Ident(pat) = &*arg.pat else {
            return no(
                arg.pat.span(),
                "name an exported function's parameter: scripts and errors call it that",
            );
        };
        let (param, ty) = (pat.ident.to_string(), &arg.ty);
        let param = param.trim_start_matches('_').to_owned();
        let held = format_ident!("__arg{}", index);
        let refused = if export.asynchronous {
            quote! {
                // SAFETY: The call's promise, unsettled, rejected over it.
                unsafe { ::zgx_module::__private::wrong_async::<#ty>(__promise, #index, #param, #held) };
                return;
            }
        } else {
            quote! {
                // SAFETY: `__out` takes why the call failed.
                return unsafe { ::zgx_module::__private::wrong::<#ty>(__out, #index, #param, #held) };
            }
        };
        reads.push(quote! {
            // SAFETY: A call's arguments, as the runtime lends them.
            let #held = unsafe { ::zgx_module::__private::arg(__args, __len, #index) };
            let #held = match <#ty as ::zgx_module::FromValue>::read(#held) {
                ::core::option::Option::Some(read) => read,
                ::core::option::Option::None => {
                    #refused
                }
            };
        });
        passed.push(quote!(#held));
        args_json.push(format!(
            "[{},{}]",
            json_string(&param),
            manifest_type(ty, records, false)?
        ));
        index += 1;
    }
    let sig = &function.sig;
    if export.asynchronous && export.action {
        return no(sig.span(), "an export is an action or async, not both");
    }
    if export.asynchronous && resolves.is_none() {
        return no(
            sig.inputs.span(),
            "an async action takes the `Promise<T>` it settles, first (after its state)",
        );
    }
    if export.asynchronous && index > 1 {
        return no(
            sig.inputs.span(),
            "an async action takes one argument besides its promise (`ui.call(name, arg)`)",
        );
    }
    if export.asynchronous && !matches!(sig.output, ReturnType::Default) {
        return no(
            sig.output.span(),
            "an async action returns nothing: it settles its promise",
        );
    }
    if export.action && index > 1 {
        return no(
            sig.inputs.span(),
            "an action takes one argument (`ui.action(name, arg)`)",
        );
    }
    // What it returns: a value, or a `Result` of one whose `Err` is raised.
    let returned = match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(&**ty),
    };
    let fallible = returned.and_then(|ty| match named(ty) {
        Some((name, args)) if name == "Result" && !args.is_empty() => Some(args[0]),
        _ => None,
    });
    let value_type = fallible.or(returned);
    let returns_json = match value_type {
        Some(ty) => manifest_type(ty, records, true)?,
        None => "\"nil\"".into(),
    };
    if export.action && returns_json != "\"nil\"" {
        return no(
            sig.output.span(),
            "an action returns nothing (or `Result<(), E>`)",
        );
    }
    let borrow = state.as_ref().map(|of| {
        let borrow = if state_mut {
            quote!(borrow_mut)
        } else {
            quote!(borrow)
        };
        quote! {
            // SAFETY: Registered with this state as its `user`.
            let __state = unsafe { ::zgx_module::__private::state::<#of>(__user) };
            #[allow(unused_mut)]
            let mut __state = __state.#borrow();
        }
    });
    let call = quote!(#ident(#(#passed),*));
    let body = match (export.action || export.asynchronous, fallible.is_some()) {
        _ if export.asynchronous => quote! { #call; },
        (false, false) => quote! {
            let __made = #call;
            // SAFETY: `__out` takes the result.
            unsafe { ::zgx_module::__private::give(__out, ::zgx_module::IntoValue::into_value(__made)) }
        },
        (false, true) => quote! {
            match #call {
                // SAFETY: `__out` takes the result, or why there is none.
                ::core::result::Result::Ok(__made) => unsafe {
                    ::zgx_module::__private::give(__out, ::zgx_module::IntoValue::into_value(__made))
                },
                ::core::result::Result::Err(__failed) => unsafe {
                    ::zgx_module::__private::fail(__out, ::zgx_module::Failed::from(__failed))
                },
            }
        },
        (true, false) => quote! {
            #call;
            ::zgx_module::abi::DONE
        },
        (true, true) => quote! {
            match #call {
                ::core::result::Result::Ok(()) => ::zgx_module::abi::DONE,
                // SAFETY: `__out` takes why it failed.
                ::core::result::Result::Err(__failed) => unsafe {
                    ::zgx_module::__private::fail(__out, ::zgx_module::Failed::from(__failed))
                },
            }
        },
    };
    let wrapper_fn = if export.asynchronous {
        quote! {
            #[doc(hidden)]
            #[allow(clippy::missing_safety_doc, unused_variables)]
            unsafe extern "C" fn #wrapper(
                __user: *mut ::zgx_module::__private::c_void,
                __args: *const ::zgx_module::abi::Raw,
                __promise: *mut ::zgx_module::__private::c_void,
            ) {
                let __len = 1usize;
                // (A panic drops the promise, which rejects it.)
                ::zgx_module::__private::guard_async(|| {
                    #(#reads)*
                    #borrow
                    #body
                })
            }
        }
    } else if export.action {
        quote! {
            #[doc(hidden)]
            #[allow(clippy::missing_safety_doc, unused_variables)]
            unsafe extern "C" fn #wrapper(
                __user: *mut ::zgx_module::__private::c_void,
                __args: *const ::zgx_module::abi::Raw,
                __out: *mut ::zgx_module::abi::Raw,
            ) -> u32 {
                let __len = 1usize;
                // SAFETY: `__out` takes why it failed, if it panicked.
                unsafe {
                    ::zgx_module::__private::guard(__out, || {
                        #(#reads)*
                        #borrow
                        #body
                    })
                }
            }
        }
    } else {
        quote! {
            #[doc(hidden)]
            #[allow(clippy::missing_safety_doc, unused_variables)]
            unsafe extern "C" fn #wrapper(
                __user: *mut ::zgx_module::__private::c_void,
                __args: *const ::zgx_module::abi::Raw,
                __len: usize,
                __out: *mut ::zgx_module::abi::Raw,
            ) -> u32 {
                // SAFETY: `__out` takes why it failed, if it panicked.
                unsafe {
                    ::zgx_module::__private::guard(__out, || {
                        #(#reads)*
                        #borrow
                        #body
                    })
                }
            }
        }
    };
    let user = match &state {
        Some(of) => quote!(::zgx_module::__private::state_user(__host.state::<#of>())),
        None => quote!(::core::ptr::null_mut()),
    };
    let raw = if export.asynchronous {
        quote!(raw_async_action)
    } else if export.action {
        quote!(raw_action)
    } else {
        quote!(raw_function)
    };
    let register = quote! {
        // SAFETY: A function this macro wrote for these arguments, and a
        // `user` that is the state it reads, or none.
        unsafe { __host.#raw(#name, #wrapper, #user, ::zgx_module::__private::keep) };
    };
    let doc = docs(&function.attrs);
    let mut manifest = format!("{}:{{", json_string(&name));
    if !export.action {
        manifest.push_str(&format!("\"member\":{},", json_string(&ident.to_string())));
    }
    manifest.push_str(&format!("\"args\":[{}]", args_json.join(",")));
    if let Some(resolves) = &resolves {
        manifest.push_str(&format!(",\"resolves\":{resolves}"));
    } else if !export.action {
        manifest.push_str(&format!(",\"returns\":{returns_json}"));
    }
    if !doc.is_empty() {
        manifest.push_str(&format!(",\"doc\":{}", json_string(&doc)));
    }
    manifest.push('}');
    Ok(Exported {
        wrapper: wrapper_fn,
        register,
        manifest,
        action: export.action,
        asynchronous: export.asynchronous,
    })
}

fn module_of(args: Tokens, mut item: ItemMod) -> Result<Tokens> {
    let mut name = None;
    syn::parse::Parser::parse2(
        syn::meta::parser(|meta| {
            if meta.path.is_ident("name") {
                name = Some(meta.value()?.parse::<LitStr>()?.value());
                Ok(())
            } else {
                Err(meta.error("#[module] takes `name = \"…\"`"))
            }
        }),
        args,
    )?;
    let name = name.unwrap_or_else(|| item.ident.to_string());
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(Error::new(
            item.ident.span(),
            "a module's name is letters, digits and `_`: scripts write it as `ui.module(\"name\")`",
        ));
    }
    let Some((_, items)) = &mut item.content else {
        return Err(Error::new(
            item.span(),
            "#[module] goes on a `mod` with its body: `mod name { … }`",
        ));
    };
    // The module's records first: a function's types name them.
    let mut records = Vec::new();
    let mut records_json = Vec::new();
    for item in items.iter() {
        let Item::Struct(record) = item else { continue };
        if !derives_record(&record.attrs) {
            continue;
        }
        records.push(record.ident.to_string());
    }
    for item in items.iter() {
        let Item::Struct(record) = item else { continue };
        if !derives_record(&record.attrs) {
            continue;
        }
        let Fields::Named(fields) = &record.fields else {
            continue; // (The derive says what is wrong with it.)
        };
        let mut fields_json = Vec::new();
        for field in &fields.named {
            let key = field
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            fields_json.push(format!(
                "{}:{}",
                json_string(&key),
                manifest_type(&field.ty, &records, true)?
            ));
        }
        records_json.push(format!(
            "{}:{{{}}}",
            json_string(&record.ident.to_string()),
            fields_json.join(",")
        ));
    }
    let mut all = Vec::new();
    for item in items.iter_mut() {
        let Item::Fn(function) = item else { continue };
        let Some(at) = function
            .attrs
            .iter()
            .position(|attr| is_attr(attr, "export"))
        else {
            continue;
        };
        let export = export_of(&function.attrs[at])?;
        let (path, marker) = (
            function.attrs[at].path().clone(),
            format_ident!("{}", IN_MODULE),
        );
        function.attrs[at] = syn::parse_quote!(#[#path(#marker)]);
        all.push(exported(function, export, &name, &records)?);
    }
    let list = |action: bool, asynchronous: bool| {
        all.iter()
            .filter(|e| e.action == action && e.asynchronous == asynchronous)
            .map(|e| e.manifest.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let json = format!(
        "{{\"zgx_module\":1,\"abi\":{ABI},\"name\":{},\"version\":{},\"records\":{{{}}},\"functions\":{{{}}},\"actions\":{{{}}},\"async\":{{{}}}}}",
        json_string(&name),
        json_string(&version),
        records_json.join(","),
        list(false, false),
        list(true, false),
        list(false, true),
    );
    let size = 32 + json.len() + 16;
    // (`zgx_module::abi::ENTRY` and `MANIFEST`, and the name.)
    let entry_symbol = format!("zgx_module_v1_{name}");
    let manifest_symbol = format!("zgx_module_manifest_{name}");
    let wrappers = all.iter().map(|e| &e.wrapper);
    let registers = all.iter().map(|e| &e.register);
    let generated: Vec<Item> = vec![
        syn::parse2(quote! {
            const _: () = assert!(
                ::zgx_module::abi::ABI == #ABI,
                "zgx-module-macros and zgx-module are of different versions"
            );
        })?,
        syn::parse2(quote! {
            /// The module's manifest (`zgx_module::abi::MANIFEST`): read by
            /// the runtime as it loads the library, and by tools in its file.
            #[doc(hidden)]
            #[used]
            #[unsafe(export_name = #manifest_symbol)]
            pub static zgx_module_manifest: [u8; #size] = ::zgx_module::abi::manifest(#json);
        })?,
        syn::parse2(quote! {
            /// # Safety
            /// Called by the zgx runtime alone (`zgx_module::abi::Entry`).
            #[doc(hidden)]
            #[unsafe(export_name = #entry_symbol)]
            pub unsafe extern "C" fn zgx_module_v1(
                api: *const ::zgx_module::abi::Api,
                registrar: *const ::zgx_module::abi::Registrar,
            ) -> u32 {
                // SAFETY: As the runtime calls it.
                unsafe {
                    ::zgx_module::enter(api, registrar, |__host: &::zgx_module::Host| {
                        #(#registers)*
                    })
                }
            }
        })?,
    ];
    for wrapper in wrappers {
        items.push(syn::parse2(wrapper.clone())?);
    }
    items.extend(generated);
    Ok(quote!(#item))
}

fn record_of(item: &ItemStruct) -> Result<Tokens> {
    let ident = &item.ident;
    let Fields::Named(fields) = &item.fields else {
        return Err(Error::new(
            item.span(),
            "a record is a struct of named fields: a script passes a map of them",
        ));
    };
    let mut lifetime = None;
    for param in &item.generics.params {
        match param {
            GenericParam::Lifetime(param) if lifetime.is_none() => {
                lifetime = Some(param.lifetime.clone());
            }
            _ => {
                return Err(Error::new(
                    param.span(),
                    "a record has at most one lifetime (what it borrows from its argument) and no other parameter",
                ));
            }
        }
    }
    let (own, lt) = match &lifetime {
        Some(lt) => (quote!(<#lt>), lt.clone()),
        None => (quote!(), syn::Lifetime::new("'__zgx", Span::call_site())),
    };
    let names: Vec<_> = fields
        .named
        .iter()
        .map(|f| f.ident.clone().unwrap())
        .collect();
    let keys: Vec<_> = names.iter().map(|n| n.to_string()).collect();
    let types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();
    let held: Vec<_> = (0..names.len())
        .map(|i| format_ident!("__f{}", i))
        .collect();
    let count = names.len();
    let kind = format!("a map ({ident})");
    let fields_of = quote! {
        let [#(#held),*]: [::zgx_module::Ref<#lt>; #count] = __value
            .get([#(#keys),*])
            .map(|field| field.unwrap_or(::zgx_module::Ref::NIL));
    };
    Ok(quote! {
        impl<#lt> ::zgx_module::FromValue<#lt> for #ident #own {
            const KIND: &'static str = #kind;
            #[inline]
            fn read(__value: ::zgx_module::Ref<#lt>) -> ::core::option::Option<Self> {
                if !__value.is_map() {
                    return ::core::option::Option::None;
                }
                #fields_of
                ::core::option::Option::Some(#ident {
                    #(#names: <#types as ::zgx_module::FromValue<#lt>>::read(#held)?,)*
                })
            }
            #[inline]
            fn is(__value: ::zgx_module::Ref<#lt>) -> bool {
                if !__value.is_map() {
                    return false;
                }
                #fields_of
                true #(&& <#types as ::zgx_module::FromValue<#lt>>::is(#held))*
            }
            #[cold]
            fn wrong(__value: ::zgx_module::Ref<#lt>) -> ::std::string::String {
                if !__value.is_map() {
                    return ::std::format!("must be {}, not {}", #kind, __value.kind());
                }
                #fields_of
                #(if !<#types as ::zgx_module::FromValue<#lt>>::is(#held) {
                    return ::std::format!(
                        "field `{}` {}",
                        #keys,
                        <#types as ::zgx_module::FromValue<#lt>>::wrong(#held)
                    );
                })*
                ::std::format!("must be {}", #kind)
            }
        }
        impl #own ::zgx_module::IntoValue for #ident #own {
            fn into_value(self) -> ::zgx_module::Value {
                ::zgx_module::Value::record([
                    #((#keys, ::zgx_module::IntoValue::into_value(self.#names)),)*
                ])
            }
        }
    })
}

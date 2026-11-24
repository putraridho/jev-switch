//! The type-file rule (DESIGN 8.10): no type that can cross to the UI, and no frozen domain type,
//! has a field that could hold free text.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::path::{Path, PathBuf};
use syn::{Attribute, Fields, GenericArgument, Item, Meta, PathArguments, ReturnType, Type};

/// The frozen type files: every struct and enum in them is checked.
const TYPE_FILES: [&str; 9] = [
    "time.rs",
    "quantity.rs",
    "label.rs",
    "provider.rs",
    "meter.rs",
    "usage.rs",
    "port.rs",
    "secret.rs",
    "ipc.rs",
];

/// Type names a field may not mention.
const BANNED: [&str; 9] = [
    "String", "str", "Cow", "OsString", "OsStr", "PathBuf", "Path", "Vec<u8>", "Value",
];

/// The two exceptions, as `(file, type, field)`.
const EXCEPTIONS: [(&str, &str, &str); 2] = [
    ("label.rs", "Label", "0"),
    ("ipc.rs", "Diagnostics", "app_version"),
];

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every type name `ty` mentions, at any depth; `Vec<u8>` is reported as one name.
fn mentions(ty: &Type, out: &mut Vec<String>) {
    match ty {
        Type::Path(p) => {
            if let Some(q) = &p.qself {
                mentions(&q.ty, out);
            }
            path_mentions(&p.path, out);
        }
        Type::Reference(r) => mentions(&r.elem, out),
        Type::Ptr(p) => mentions(&p.elem, out),
        Type::Slice(s) => mentions(&s.elem, out),
        Type::Array(a) => mentions(&a.elem, out),
        Type::Paren(p) => mentions(&p.elem, out),
        Type::Group(g) => mentions(&g.elem, out),
        Type::Tuple(t) => t.elems.iter().for_each(|e| mentions(e, out)),
        Type::BareFn(f) => {
            f.inputs.iter().for_each(|arg| mentions(&arg.ty, out));
            if let ReturnType::Type(_, ty) = &f.output {
                mentions(ty, out);
            }
        }
        Type::TraitObject(t) => {
            for bound in &t.bounds {
                if let syn::TypeParamBound::Trait(tb) = bound {
                    path_mentions(&tb.path, out);
                }
            }
        }
        Type::ImplTrait(t) => {
            for bound in &t.bounds {
                if let syn::TypeParamBound::Trait(tb) = bound {
                    path_mentions(&tb.path, out);
                }
            }
        }
        // A type this walk cannot see into is reported, so it fails instead of passing unseen.
        _ => out.push("<opaque>".to_owned()),
    }
}

fn path_mentions(path: &syn::Path, out: &mut Vec<String>) {
    for segment in &path.segments {
        let name = segment.ident.to_string();
        match &segment.arguments {
            PathArguments::None => {}
            PathArguments::AngleBracketed(args) => {
                for arg in &args.args {
                    match arg {
                        GenericArgument::Type(inner) => {
                            if name == "Vec" && is_u8(inner) {
                                out.push("Vec<u8>".to_owned());
                            }
                            mentions(inner, out);
                        }
                        GenericArgument::AssocType(assoc) => mentions(&assoc.ty, out),
                        _ => {}
                    }
                }
            }
            PathArguments::Parenthesized(args) => {
                args.inputs.iter().for_each(|input| mentions(input, out));
                if let ReturnType::Type(_, ty) = &args.output {
                    mentions(ty, out);
                }
            }
        }
        out.push(name);
    }
}

fn is_u8(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("u8"))
}

/// True when the attributes derive `TS` (written `cfg_attr(feature = "bindings", derive(ts_rs::TS))`).
fn derives_ts(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        Meta::List(list) => {
            let tokens = list.tokens.to_string();
            let words: Vec<&str> = tokens
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .collect();
            (list.path.is_ident("derive") || list.path.is_ident("cfg_attr"))
                && words.contains(&"TS")
        }
        Meta::Path(_) | Meta::NameValue(_) => false,
    })
}

/// `(type, field, banned name)` for every offending field of the struct or enum `item`.
fn field_findings(type_name: &str, fields: &Fields, out: &mut Vec<(String, String, String)>) {
    for (index, field) in fields.iter().enumerate() {
        let field_name = field
            .ident
            .as_ref()
            .map_or_else(|| index.to_string(), ToString::to_string);
        let mut names = Vec::new();
        mentions(&field.ty, &mut names);
        for name in names {
            if BANNED.contains(&name.as_str()) || name == "<opaque>" {
                out.push((type_name.to_owned(), field_name.clone(), name));
            }
        }
    }
}

/// The findings of one source text. `whole_file` checks every type; otherwise only `TS` types.
fn findings(source: &str, whole_file: bool) -> Vec<(String, String, String)> {
    let file = syn::parse_file(source).unwrap();
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            Item::Struct(s) if whole_file || derives_ts(&s.attrs) => {
                field_findings(&s.ident.to_string(), &s.fields, &mut out);
            }
            Item::Enum(e) if whole_file || derives_ts(&e.attrs) => {
                for variant in &e.variants {
                    field_findings(&e.ident.to_string(), &variant.fields, &mut out);
                }
            }
            Item::Union(u) => out.push((
                u.ident.to_string(),
                "<union>".to_owned(),
                "<opaque>".to_owned(),
            )),
            _ => {}
        }
    }
    out
}

#[test]
fn no_type_can_hold_a_free_string_except_label_and_the_app_version() {
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);
    files.sort();
    let mut seen_type_files = Vec::new();
    let mut seen_exceptions = Vec::new();
    let mut offences = Vec::new();
    let mut ts_types = 0_usize;
    for path in &files {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let top_level = path.parent().unwrap() == src_dir();
        let whole_file = top_level && TYPE_FILES.contains(&name.as_str());
        if whole_file {
            seen_type_files.push(name.clone());
        }
        let source = std::fs::read_to_string(path).unwrap();
        ts_types += source.matches("derive(ts_rs::TS)").count();
        for (type_name, field, banned) in findings(&source, whole_file) {
            let exception = EXCEPTIONS
                .iter()
                .find(|(f, t, fld)| *f == name && *t == type_name && *fld == field);
            match exception {
                Some(e) => seen_exceptions.push(*e),
                None => offences.push(format!("{name}: {type_name}.{field} mentions {banned}")),
            }
        }
    }
    assert!(
        offences.is_empty(),
        "string-like fields:\n{}",
        offences.join("\n")
    );
    seen_type_files.sort();
    let mut expected: Vec<String> = TYPE_FILES.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    assert_eq!(seen_type_files, expected, "a frozen type file is missing");
    // Both exceptions are real (each matched at least once), and the IPC types were seen.
    for exception in EXCEPTIONS {
        assert!(
            seen_exceptions.contains(&exception),
            "exception no longer needed: {exception:?}"
        );
    }
    assert!(ts_types >= 32, "only {ts_types} types derive TS");
}

#[test]
fn the_ipc_types_outside_ipc_rs_are_found_by_their_derive() {
    for (file, type_name) in [
        ("settings.rs", "Settings"),
        ("settings.rs", "SettingsPatch"),
        ("assess.rs", "Severity"),
        ("assess.rs", "Phase"),
    ] {
        let source = std::fs::read_to_string(src_dir().join(file)).unwrap();
        let parsed = syn::parse_file(&source).unwrap();
        let found = parsed.items.iter().any(|item| match item {
            Item::Struct(s) => s.ident == type_name && derives_ts(&s.attrs),
            Item::Enum(e) => e.ident == type_name && derives_ts(&e.attrs),
            _ => false,
        });
        assert!(found, "{file}: {type_name} does not derive TS");
    }
}

#[test]
fn the_rule_catches_each_banned_shape() {
    let cases = [
        ("pub struct A { pub a: String }", "String"),
        ("pub struct A { pub a: &'static str }", "str"),
        ("pub struct A { pub a: Option<String> }", "String"),
        ("pub struct A(pub Box<str>);", "str"),
        (
            "pub struct A { pub a: std::borrow::Cow<'static, str> }",
            "Cow",
        ),
        ("pub struct A { pub a: std::ffi::OsString }", "OsString"),
        ("pub struct A { pub a: std::path::PathBuf }", "PathBuf"),
        ("pub struct A { pub a: Vec<u8> }", "Vec<u8>"),
        ("pub struct A { pub a: serde_json::Value }", "Value"),
        (
            "pub struct A { pub a: Vec<(u8, Option<Box<String>>)> }",
            "String",
        ),
        ("pub struct A { pub a: [String; 2] }", "String"),
        ("pub struct A { pub a: Box<dyn Fn() -> String> }", "String"),
        ("pub struct A { pub a: fn(&str) }", "str"),
        ("pub enum A { B(String) }", "String"),
        (
            "pub enum A { B { c: Option<std::path::PathBuf> } }",
            "PathBuf",
        ),
        (
            "pub struct A { pub a: std::collections::HashMap<u8, String> }",
            "String",
        ),
    ];
    for (source, banned) in cases {
        let found = findings(source, true);
        assert!(
            found.iter().any(|(_, _, name)| name == banned),
            "{source}: {found:?}"
        );
    }
    // Outside a type file only `TS` types are checked.
    assert!(findings("pub struct A { pub a: String }", false).is_empty());
    let ts =
        r#"#[cfg_attr(feature = "bindings", derive(ts_rs::TS))] pub struct A { pub a: String }"#;
    assert_eq!(findings(ts, false).len(), 1);
    let ts_enum = "#[derive(Clone, TS)] pub enum A { B { c: String } }";
    assert_eq!(findings(ts_enum, false).len(), 1);
    // And the shapes the domain does use pass.
    let fine = "pub struct A { pub a: Vec<u16>, pub b: Option<u64>, pub c: [u8; 4], pub d: Box<dyn Fn() -> u8>, pub e: Label }";
    assert!(findings(fine, true).is_empty());
}

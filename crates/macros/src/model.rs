//! Parsed derive input: the container, its fields or variants, and their
//! `#[semantic(...)]` attributes.

use syn::ext::IdentExt;
use syn::spanned::Spanned;

pub struct Container {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    pub shape: Shape,
}

pub enum Shape {
    /// A struct with named fields: a record.
    Record(Vec<Field>),
    /// A single-field tuple struct, represented as its field.
    Newtype(syn::Type),
    /// An enum of unit variants, represented as strings.
    Enum(Vec<Variant>),
    /// An enum represented as an object with the variant name in `tag`.
    Tagged { tag: String, variants: Vec<Variant> },
}

pub struct Field {
    pub ident: syn::Ident,
    pub ty: syn::Type,
    pub name: String,
    pub doc: Option<String>,
    pub kind: FieldKind,
}

pub enum FieldKind {
    /// Must be present.
    Required,
    /// An `Option` field: may be missing, omitted when `None`.
    Optional,
    /// An `Option` field marked `required`: always present, possibly null.
    Nullable,
    /// May be missing, then filled from `Default::default()` or the given function.
    Default(Option<syn::ExprPath>),
    /// The fields of a nested record, inlined.
    Flatten,
}

pub struct Variant {
    pub ident: syn::Ident,
    pub name: String,
    pub doc: Option<String>,
    /// `None` for unit variants.
    pub fields: Option<Vec<Field>>,
}

#[derive(Default)]
struct ContainerAttrs {
    rename_all: bool,
    tag: Option<String>,
}

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    default: Option<Option<syn::ExprPath>>,
    required: bool,
    flatten: bool,
}

impl Container {
    pub fn parse(input: syn::DeriveInput) -> syn::Result<Self> {
        let attrs = container_attrs(&input.attrs)?;
        let shape = match input.data {
            syn::Data::Struct(data) => {
                if attrs.tag.is_some() {
                    return Err(syn::Error::new(
                        input.ident.span(),
                        "`tag` is only supported on enums",
                    ));
                }
                match data.fields {
                    syn::Fields::Named(fields) => {
                        Shape::Record(parse_fields(fields.named.into_iter(), attrs.rename_all)?)
                    }
                    syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        Shape::Newtype(fields.unnamed.into_iter().next().expect("one field").ty)
                    }
                    fields => {
                        return Err(syn::Error::new(
                            fields.span(),
                            "only structs with named fields and newtype structs are supported",
                        ));
                    }
                }
            }
            syn::Data::Enum(data) => {
                let variants = data
                    .variants
                    .into_iter()
                    .map(|variant| parse_variant(variant, attrs.rename_all))
                    .collect::<syn::Result<Vec<_>>>()?;
                match attrs.tag {
                    Some(tag) => Shape::Tagged { tag, variants },
                    None => {
                        if let Some(variant) = variants.iter().find(|v| v.fields.is_some()) {
                            return Err(syn::Error::new(
                                variant.ident.span(),
                                "enums with fields need #[semantic(tag = \"...\")]",
                            ));
                        }
                        Shape::Enum(variants)
                    }
                }
            }
            syn::Data::Union(data) => {
                return Err(syn::Error::new(
                    data.union_token.span(),
                    "unions are not supported",
                ));
            }
        };
        Ok(Self {
            ident: input.ident,
            generics: input.generics,
            shape,
        })
    }
}

fn parse_variant(variant: syn::Variant, rename_all: bool) -> syn::Result<Variant> {
    let attrs = field_attrs(&variant.attrs)?;
    if attrs.default.is_some() || attrs.required || attrs.flatten {
        return Err(syn::Error::new(
            variant.ident.span(),
            "variants only support `rename`",
        ));
    }
    let fields = match variant.fields {
        syn::Fields::Unit => None,
        syn::Fields::Named(fields) => Some(parse_fields(fields.named.into_iter(), rename_all)?),
        syn::Fields::Unnamed(fields) => {
            return Err(syn::Error::new(
                fields.span(),
                "tuple variants are not supported",
            ));
        }
    };
    Ok(Variant {
        name: attrs
            .rename
            .unwrap_or_else(|| rename(&variant.ident, rename_all)),
        doc: doc(&variant.attrs),
        ident: variant.ident,
        fields,
    })
}

fn parse_fields(
    fields: impl Iterator<Item = syn::Field>,
    rename_all: bool,
) -> syn::Result<Vec<Field>> {
    fields
        .map(|field| {
            let ident = field.ident.clone().expect("named field");
            let attrs = field_attrs(&field.attrs)?;
            let option = is_option(&field.ty);
            let exclusive = [attrs.default.is_some(), attrs.required, attrs.flatten];
            if exclusive.into_iter().filter(|set| *set).count() > 1 {
                return Err(syn::Error::new(
                    ident.span(),
                    "`default`, `required` and `flatten` are mutually exclusive",
                ));
            }
            if attrs.flatten && attrs.rename.is_some() {
                return Err(syn::Error::new(
                    ident.span(),
                    "flattened fields can't be renamed",
                ));
            }
            let kind = if attrs.flatten {
                FieldKind::Flatten
            } else if let Some(default) = attrs.default {
                FieldKind::Default(default)
            } else if attrs.required {
                if !option {
                    return Err(syn::Error::new(
                        ident.span(),
                        "`required` only applies to `Option` fields",
                    ));
                }
                FieldKind::Nullable
            } else if option {
                FieldKind::Optional
            } else {
                FieldKind::Required
            };
            Ok(Field {
                name: attrs.rename.unwrap_or_else(|| rename(&ident, rename_all)),
                doc: doc(&field.attrs),
                ident,
                ty: field.ty,
                kind,
            })
        })
        .collect()
}

fn semantic_attrs(attrs: &[syn::Attribute]) -> impl Iterator<Item = &syn::Attribute> {
    attrs.iter().filter(|attr| attr.path().is_ident("semantic"))
}

fn container_attrs(attrs: &[syn::Attribute]) -> syn::Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();
    for attr in semantic_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value: syn::LitStr = meta.value()?.parse()?;
                if value.value() != "snake_case" {
                    return Err(meta.error("only `rename_all = \"snake_case\"` is supported"));
                }
                out.rename_all = true;
            } else if meta.path.is_ident("tag") {
                let value: syn::LitStr = meta.value()?.parse()?;
                out.tag = Some(value.value());
            } else {
                return Err(meta.error("unsupported container attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in semantic_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: syn::LitStr = meta.value()?.parse()?;
                out.rename = Some(value.value());
            } else if meta.path.is_ident("default") {
                out.default = Some(if meta.input.peek(syn::Token![=]) {
                    let value: syn::LitStr = meta.value()?.parse()?;
                    Some(value.parse()?)
                } else {
                    None
                });
            } else if meta.path.is_ident("required") {
                out.required = true;
            } else if meta.path.is_ident("flatten") {
                out.flatten = true;
            } else {
                return Err(meta.error("unsupported field attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// The joined doc comment lines, if any.
fn doc(attrs: &[syn::Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(value),
                        ..
                    }),
                ..
            }) => Some(value.value().trim().to_owned()),
            _ => None,
        })
        .collect();
    let doc = lines.join("\n").trim().to_owned();
    (!doc.is_empty()).then_some(doc)
}

fn rename(ident: &syn::Ident, snake_case: bool) -> String {
    let name = ident.unraw().to_string();
    if !snake_case {
        return name;
    }
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Whether the type is syntactically an `Option<T>`.
fn is_option(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => {
            path.path.segments.last().is_some_and(|segment| {
                segment.ident == "Option"
                    && matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args) if args.args.len() == 1)
            })
        }
        _ => false,
    }
}

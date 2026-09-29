//! Derive macros for the semantic data model traits `SemanticType`,
//! `IntoValue` and `FromValue` of `semantic_data::value`, plus `Class`, which
//! implements all three for class instances along with
//! `semantic_data::attr::ClassDescriptorConst`.
//!
//! Supported shapes:
//! - structs with named fields: records, encoded as objects keyed by the plain
//!   field names. Unknown fields are ignored when decoding.
//!
//! `#[derive(Class)]` takes a struct with named fields and
//! `#[semantic(id = "...")]`: a class instance (entity), keyed by attribute ids
//! like DB query results. See below.
//! - newtype structs: transparent.
//! - enums of unit variants: string enums.
//! - enums with `#[semantic(tag = "field")]`: objects with the variant name in
//!   `field`, plus the fields of struct variants.
//!
//! Attributes, under `#[semantic(...)]`:
//! - container: `rename_all = "snake_case"`, `tag = "..."` (enums),
//!   `id = "..."` and `namespace = "..."` (classes)
//! - field: `rename = "..."`, `default`, `default = "path::to::fn"`, `flatten`,
//!   `required` on `Option` fields, and `attr = Marker` on class fields
//! - variant: `rename = "..."`
//!
//! Class fields are keyed `<namespace>:<name>`, where the namespace defaults to
//! the class id; `attr = Marker` keys a field by `Marker::ID` instead (see
//! `semantic_data::attr!`), like the built-in `id`. Decoding also accepts the
//! plain name (`Marker::PLAIN_NAME` or the field name) as an alias; both at once
//! are an error. Encoding writes the built-in `type` as the class id; decoding
//! accepts a missing `type` but rejects a different one. The `SemanticType` of a
//! class is a record keyed by the attribute ids, plus an optional `type` that
//! defaults to the class id. `id`, `namespace` and `attr` are only allowed on
//! classes, and class fields can't be flattened.
//!
//! `Option<T>` fields are optional: missing and null decode to `None`, and
//! `None` is omitted. With `required` the field is always encoded, `None` as null,
//! and must be present. `default` fields may be missing. Doc comments become the
//! field or variant description.

mod model;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use model::{Container, Field, FieldKind, Key, Shape, Variant};

#[proc_macro_derive(SemanticType, attributes(semantic))]
pub fn derive_semantic_type(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, Container::parse, semantic_type)
}

#[proc_macro_derive(IntoValue, attributes(semantic))]
pub fn derive_into_value(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, Container::parse, into_value)
}

#[proc_macro_derive(FromValue, attributes(semantic))]
pub fn derive_from_value(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, Container::parse, from_value)
}

#[proc_macro_derive(Class, attributes(semantic))]
pub fn derive_class(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, Container::parse_class, |container| {
        let mut out = semantic_type(container);
        out.extend(into_value(container));
        out.extend(from_value(container));
        out.extend(class_descriptor(container));
        out
    })
}

fn expand(
    input: proc_macro::TokenStream,
    parse: fn(syn::DeriveInput) -> syn::Result<Container>,
    body: fn(&Container) -> TokenStream,
) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match parse(input) {
        Ok(container) => body(&container).into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn private() -> TokenStream {
    quote!(::semantic_data::value::convert::__private)
}

fn option_str(value: &Option<String>) -> TokenStream {
    match value {
        Some(value) => quote!(::core::option::Option::Some(#value)),
        None => quote!(::core::option::Option::None),
    }
}

impl Key {
    /// The key expression.
    fn id(&self) -> TokenStream {
        match self {
            Key::Plain(name) => quote!(#name),
            Key::Qualified { id, .. } => quote!(#id),
            Key::Attr(path) => quote!(<#path as ::semantic_data::attr::AttrDescriptorConst>::ID),
        }
    }

    /// The alias expression, equal to the key for plain keys.
    fn alias(&self) -> TokenStream {
        match self {
            Key::Plain(name) => quote!(#name),
            Key::Qualified { alias, .. } => quote!(#alias),
            Key::Attr(path) => {
                quote!(<#path as ::semantic_data::attr::AttrDescriptorConst>::PLAIN_NAME)
            }
        }
    }
}

impl Field {
    fn key(&self) -> &Key {
        self.key
            .as_ref()
            .expect("only flattened fields have no key")
    }
}

fn class_descriptor(container: &Container) -> TokenStream {
    let Shape::Struct {
        class: Some(class), ..
    } = &container.shape
    else {
        unreachable!("classes are parsed with an id")
    };
    let ident = &container.ident;
    let (impl_generics, ty_generics, where_clause) = container.generics.split_for_impl();
    quote! {
        impl #impl_generics ::semantic_data::attr::ClassDescriptorConst for #ident #ty_generics #where_clause {
            const ID: &'static str = #class;
        }
    }
}

fn semantic_type(container: &Container) -> TokenStream {
    let p = private();
    let body = match &container.shape {
        Shape::Struct {
            class: None,
            fields,
        } => {
            let (defs, flattened) = field_defs(fields);
            quote!(#p::record_type(::std::vec![#(#defs),*], ::std::vec![#(#flattened),*]))
        }
        Shape::Struct {
            class: Some(class),
            fields,
        } => {
            let (defs, _) = field_defs(fields);
            quote!(#p::class_type(#class, ::std::vec![#(#defs),*]))
        }
        Shape::Newtype(ty) => {
            quote!(<#ty as ::semantic_data::value::SemanticType>::semantic_type())
        }
        Shape::Enum(variants) => {
            let variants = variants.iter().map(|variant| {
                let name = &variant.name;
                let doc = option_str(&variant.doc);
                quote!((#name, #doc))
            });
            quote!(#p::enum_type(::std::vec![#(#variants),*]))
        }
        Shape::Tagged { tag, variants } => {
            let variants = variants.iter().map(|variant| {
                let name = &variant.name;
                let doc = option_str(&variant.doc);
                let record = match &variant.fields {
                    Some(fields) => {
                        let (defs, flattened) = field_defs(fields);
                        quote!(::core::option::Option::Some(#p::record(
                            ::std::vec![#(#defs),*],
                            ::std::vec![#(#flattened),*],
                        )))
                    }
                    None => quote!(::core::option::Option::None),
                };
                quote!((#name, #doc, #record))
            });
            quote!(#p::tagged_type(#tag, ::std::vec![#(#variants),*]))
        }
    };
    let ident = &container.ident;
    let (impl_generics, ty_generics, where_clause) = container.generics.split_for_impl();
    quote! {
        impl #impl_generics ::semantic_data::value::SemanticType for #ident #ty_generics #where_clause {
            fn semantic_type() -> ::semantic_data::schema::Type {
                #body
            }
        }
    }
}

/// The `FieldDef`s of the own fields, and the field lists of flattened fields.
fn field_defs(fields: &[Field]) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let p = private();
    let mut defs = Vec::new();
    let mut flattened = Vec::new();
    for field in fields {
        let ty = &field.ty;
        if let FieldKind::Flatten = field.kind {
            flattened.push(quote!(#p::flattened::<#ty>()));
            continue;
        }
        let name = field.key().id();
        let doc = option_str(&field.doc);
        let required = matches!(field.kind, FieldKind::Required | FieldKind::Nullable);
        let default = match &field.kind {
            FieldKind::Default(None) => quote!(#p::default_value::<#ty>()),
            FieldKind::Default(Some(path)) => quote!(::core::option::Option::Some(
                ::semantic_data::value::IntoValue::into_value(#path())
            )),
            _ => quote!(::core::option::Option::None),
        };
        defs.push(quote! {
            #p::FieldDef {
                name: #name,
                ty: <#ty as ::semantic_data::value::SemanticType>::semantic_type(),
                required: #required,
                default: #default,
                description: #doc,
            }
        });
    }
    (defs, flattened)
}

fn into_value(container: &Container) -> TokenStream {
    let p = private();
    let body = match &container.shape {
        Shape::Struct { class, fields } => {
            let inserts = field_inserts(fields, |ident| quote!(self.#ident));
            let object = match class {
                Some(class) => quote!(#p::class_object(#class)),
                None => quote!(#p::Object::new()),
            };
            quote! {
                #[allow(unused_mut)]
                let mut __object = #object;
                #(#inserts)*
                #p::Value::Object(__object)
            }
        }
        Shape::Newtype(_) => quote!(::semantic_data::value::IntoValue::into_value(self.0)),
        Shape::Enum(variants) => {
            let arms = variants.iter().map(|Variant { ident, name, .. }| {
                quote!(Self::#ident => #p::Value::String(::std::string::String::from(#name)))
            });
            quote!(match self { #(#arms),* })
        }
        Shape::Tagged { tag, variants } => {
            let arms = variants.iter().map(|variant| {
                let ident = &variant.ident;
                let name = &variant.name;
                let fields = variant.fields.as_deref().unwrap_or_default();
                let bindings = fields.iter().map(|field| &field.ident);
                let inserts = field_inserts(fields, |ident| quote!(#ident));
                quote! {
                    Self::#ident { #(#bindings),* } => {
                        let mut __object = #p::Object::new();
                        #(#inserts)*
                        __object.insert(#tag, #p::Value::String(::std::string::String::from(#name)));
                        #p::Value::Object(__object)
                    }
                }
            });
            quote!(match self { #(#arms)* })
        }
    };
    let ident = &container.ident;
    let (impl_generics, ty_generics, where_clause) = container.generics.split_for_impl();
    quote! {
        impl #impl_generics ::semantic_data::value::IntoValue for #ident #ty_generics #where_clause {
            fn into_value(self) -> ::semantic_data::value::Value {
                #body
            }
        }
    }
}

/// Statements inserting the fields into `__object`, flattened fields first.
fn field_inserts(
    fields: &[Field],
    access: impl Fn(&syn::Ident) -> TokenStream,
) -> Vec<TokenStream> {
    let p = private();
    let (flattened, own): (Vec<&Field>, Vec<&Field>) = fields
        .iter()
        .partition(|field| matches!(field.kind, FieldKind::Flatten));
    flattened
        .into_iter()
        .chain(own)
        .map(|field| {
            let value = access(&field.ident);
            if let FieldKind::Flatten = field.kind {
                return quote! {
                    #p::merge(&mut __object, ::semantic_data::value::IntoValue::into_value(#value));
                };
            }
            let name = field.key().id();
            match field.kind {
                FieldKind::Flatten => unreachable!("handled above"),
                FieldKind::Optional => quote! {
                    if let ::core::option::Option::Some(__value) = #value {
                        __object.insert(#name, ::semantic_data::value::IntoValue::into_value(__value));
                    }
                },
                _ => quote! {
                    __object.insert(#name, ::semantic_data::value::IntoValue::into_value(#value));
                },
            }
        })
        .collect()
}

fn from_value(container: &Container) -> TokenStream {
    let p = private();
    let body = match &container.shape {
        Shape::Struct { class, fields } => {
            let construct = construct(quote!(Self), fields);
            let check = class
                .as_ref()
                .map(|class| quote!(#p::check_class(&mut __object, #class)?;));
            quote! {
                #[allow(unused_mut)]
                let mut __object = #p::object(value)?;
                #check
                ::core::result::Result::Ok(#construct)
            }
        }
        Shape::Newtype(ty) => quote! {
            <#ty as ::semantic_data::value::FromValue>::from_value(value).map(Self)
        },
        Shape::Enum(variants) => {
            let names = variants.iter().map(|variant| &variant.name);
            let arms = variants.iter().map(|Variant { ident, name, .. }| {
                quote!(#name => ::core::result::Result::Ok(Self::#ident))
            });
            quote! {
                let __name = #p::string(value)?;
                match __name.as_str() {
                    #(#arms,)*
                    __other => ::core::result::Result::Err(
                        #p::unknown_variant(__other, &[#(#names),*])
                    ),
                }
            }
        }
        Shape::Tagged { tag, variants } => {
            let names = variants.iter().map(|variant| &variant.name);
            let arms = variants.iter().map(|variant| {
                let ident = &variant.ident;
                let name = &variant.name;
                let construct = match &variant.fields {
                    Some(fields) => construct(quote!(Self::#ident), fields),
                    None => quote!(Self::#ident),
                };
                quote!(#name => ::core::result::Result::Ok(#construct))
            });
            quote! {
                let mut __object = #p::object(value)?;
                let __tag: ::std::string::String = #p::required(&mut __object, #tag, #tag)?;
                match __tag.as_str() {
                    #(#arms,)*
                    __other => ::core::result::Result::Err(
                        #p::unknown_variant(__other, &[#(#names),*]).in_field(#tag)
                    ),
                }
            }
        }
    };
    let ident = &container.ident;
    let (impl_generics, ty_generics, where_clause) = container.generics.split_for_impl();
    quote! {
        impl #impl_generics ::semantic_data::value::FromValue for #ident #ty_generics #where_clause {
            fn from_value(
                value: ::semantic_data::value::Value,
            ) -> ::core::result::Result<Self, ::semantic_data::value::FromValueError> {
                #body
            }
        }
    }
}

/// An expression building `path { fields }` from `__object`. Own fields are
/// taken first, so flattened fields decode from the remaining fields.
fn construct(path: TokenStream, fields: &[Field]) -> TokenStream {
    let p = private();
    let var = |field: &Field| format_ident!("__field_{}", field.ident);
    let (flattened, own): (Vec<&Field>, Vec<&Field>) = fields
        .iter()
        .partition(|field| matches!(field.kind, FieldKind::Flatten));
    let lets = own.into_iter().chain(flattened).map(|field| {
        let var = var(field);
        let ty = &field.ty;
        if let FieldKind::Flatten = field.kind {
            return quote!(let #var = #p::flatten::<#ty>(&__object)?;);
        }
        let key = field.key();
        let (name, alias) = (key.id(), key.alias());
        let value = match &field.kind {
            FieldKind::Required | FieldKind::Nullable => {
                quote!(#p::required::<#ty>(&mut __object, #name, #alias)?)
            }
            FieldKind::Optional => quote!(#p::or_else::<#ty>(
                &mut __object, #name, #alias, || ::core::option::Option::None
            )?),
            FieldKind::Default(None) => quote!(#p::or_else::<#ty>(
                &mut __object, #name, #alias, ::core::default::Default::default
            )?),
            FieldKind::Default(Some(default)) => {
                quote!(#p::or_else::<#ty>(&mut __object, #name, #alias, #default)?)
            }
            FieldKind::Flatten => unreachable!("handled above"),
        };
        quote!(let #var = #value;)
    });
    let inits = fields.iter().map(|field| {
        let ident = &field.ident;
        let var = var(field);
        quote!(#ident: #var)
    });
    quote! {{
        #(#lets)*
        #path { #(#inits),* }
    }}
}

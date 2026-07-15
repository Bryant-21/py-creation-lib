use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DeriveInput, Expr, Fields, Lit, Meta, Token, parse_macro_input, punctuated::Punctuated,
};

/// Derive `to_hkx_object` and `from_hkx_object` for a Havok class struct.
///
/// Struct-level attribute: `#[hk_class(name = "ClassName", signature = 0xABCDEF)]`
///
/// Field-level attribute (optional): `#[hk_member(kind = "array")]`
///   - `kind = "array"` signals a `Vec<T>` field that maps to `HkxValue::Array`.
///   - Omitting the attribute uses the field type for a direct scalar mapping.
///
/// Supported field types: `i32`, `u32`, `f32`, `bool`, `String`, `Vec<f32>`,
/// `Vec<i32>`, `Vec<u32>`, `Vec<String>`.
#[proc_macro_derive(HkClass, attributes(hk_class, hk_member))]
pub fn derive_hk_class(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let struct_name = &input.ident;

    // Parse #[hk_class(name = "...", signature = 0x...)]
    let (hk_name, hk_sig) = parse_hk_class_attr(&input)?;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "HkClass only supports named-field structs",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "HkClass can only be derived for structs",
            ));
        }
    };

    let mut to_members = Vec::<TokenStream2>::new();
    let mut from_fields = Vec::<TokenStream2>::new();
    let mut field_names = Vec::<&syn::Ident>::new();

    for field in fields.iter() {
        let fname = field.ident.as_ref().unwrap();
        let fname_str = fname.to_string();
        let kind = member_kind(field);

        let to_expr = field_to_hkx_value(field, &kind)?;
        to_members.push(quote! {
            ::havok_native::hkx::model::HkxMember {
                name: #fname_str.to_string(),
                value: #to_expr,
            }
        });

        let from_expr = field_from_hkx_value(field, &fname_str, &kind)?;
        from_fields.push(from_expr);
        field_names.push(fname);
    }

    let sig_expr: TokenStream2 = {
        let v = hk_sig as u32;
        quote! { #v }
    };

    Ok(quote! {
        impl #struct_name {
            pub fn to_hkx_object(
                &self,
                name: ::std::option::Option<::std::string::String>,
            ) -> ::havok_native::hkx::model::HkxObject {
                ::havok_native::hkx::model::HkxObject {
                    name,
                    offset: 0,
                    signature: #sig_expr,
                    class_name: #hk_name.to_string(),
                    members: vec![ #(#to_members),* ],
                }
            }

            pub fn from_hkx_object(
                obj: &::havok_native::hkx::model::HkxObject,
            ) -> ::std::result::Result<Self, ::std::string::String> {
                #( #from_fields )*
                ::std::result::Result::Ok(Self {
                    #( #field_names, )*
                })
            }
        }
    })
}

/// Returns the `kind` string for a field: "array" or "scalar".
fn member_kind(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if !attr.path().is_ident("hk_member") {
            continue;
        }
        // Parse as key=value pairs: #[hk_member(kind = "array")]
        let parsed: Punctuated<Meta, Token![,]> = attr
            .parse_args_with(Punctuated::parse_terminated)
            .unwrap_or_default();
        for meta in parsed {
            if let Meta::NameValue(nv) = meta {
                if nv.path.is_ident("kind") {
                    if let Expr::Lit(expr_lit) = &nv.value {
                        if let Lit::Str(s) = &expr_lit.lit {
                            return s.value();
                        }
                    }
                }
            }
        }
    }
    "scalar".to_string()
}

/// Generate the expression that converts `self.<field>` to `HkxValue`.
fn field_to_hkx_value(field: &syn::Field, kind: &str) -> syn::Result<TokenStream2> {
    let fname = field.ident.as_ref().unwrap();

    if kind == "array" {
        // Vec<T> → HkxValue::Array(vec![HkxValue::T(elem), ...])
        let inner = vec_inner_type(field)?;
        // Iterator yields &T, so elem is a reference — pass with deref flag.
        let elem_conv = scalar_to_hkx_value_expr(&inner, quote! { elem }, true)?;
        Ok(quote! {
            ::havok_native::hkx::types::HkxValue::Array(
                self.#fname.iter().map(|elem| #elem_conv).collect()
            )
        })
    } else {
        // self.field is an owned value (Copy types) or String — no deref.
        scalar_to_hkx_value_expr(
            &type_to_scalar_kind(&field.ty)?,
            quote! { self.#fname },
            false,
        )
    }
}

/// Returns the expression that converts an HkxValue extracted from the object
/// into the Rust field value, or an error string.
fn field_from_hkx_value(field: &syn::Field, name: &str, kind: &str) -> syn::Result<TokenStream2> {
    let fname = field.ident.as_ref().unwrap();

    if kind == "array" {
        let inner = vec_inner_type(field)?;
        let elem_extract = hkx_value_extract_scalar(&inner, quote! { elem })?;
        let inner_ty = inner_type_tokens(&inner);
        Ok(quote! {
            let #fname: Vec<#inner_ty> = {
                let member = obj.members.iter().find(|m| m.name == #name)
                    .ok_or_else(|| format!("missing member: {}", #name))?;
                match &member.value {
                    ::havok_native::hkx::types::HkxValue::Array(elems) => {
                        elems.iter().map(|elem| #elem_extract).collect::<
                            ::std::result::Result<Vec<#inner_ty>, ::std::string::String>
                        >()?
                    },
                    other => return ::std::result::Result::Err(format!(
                        "member {} expected Array, got {:?}", #name, other
                    )),
                }
            };
        })
    } else {
        let scalar = type_to_scalar_kind(&field.ty)?;
        let extract = hkx_value_extract_scalar(&scalar, quote! { &member.value })?;
        let field_ty = scalar_type_tokens(&scalar);
        Ok(quote! {
            let #fname: #field_ty = {
                let member = obj.members.iter().find(|m| m.name == #name)
                    .ok_or_else(|| format!("missing member: {}", #name))?;
                #extract?
            };
        })
    }
}

// ─── Scalar kind helpers ──────────────────────────────────────────────────────

#[derive(Clone)]
enum ScalarKind {
    I32,
    U32,
    F32,
    Bool,
    StringVal,
}

fn type_to_scalar_kind(ty: &syn::Type) -> syn::Result<ScalarKind> {
    let type_str = quote! { #ty }.to_string().replace(' ', "");
    match type_str.as_str() {
        "i32" => Ok(ScalarKind::I32),
        "u32" => Ok(ScalarKind::U32),
        "f32" => Ok(ScalarKind::F32),
        "bool" => Ok(ScalarKind::Bool),
        "String" => Ok(ScalarKind::StringVal),
        _ => Err(syn::Error::new_spanned(
            ty,
            format!(
                "HkClass: unsupported scalar type '{type_str}'; use i32, u32, f32, bool, or String"
            ),
        )),
    }
}

fn scalar_type_tokens(kind: &ScalarKind) -> TokenStream2 {
    match kind {
        ScalarKind::I32 => quote! { i32 },
        ScalarKind::U32 => quote! { u32 },
        ScalarKind::F32 => quote! { f32 },
        ScalarKind::Bool => quote! { bool },
        ScalarKind::StringVal => quote! { ::std::string::String },
    }
}

fn inner_type_tokens(kind: &ScalarKind) -> TokenStream2 {
    scalar_type_tokens(kind)
}

/// `is_ref`: if true, the expression is a `&T` and needs dereferencing for Copy types.
fn scalar_to_hkx_value_expr(
    kind: &ScalarKind,
    expr: TokenStream2,
    is_ref: bool,
) -> syn::Result<TokenStream2> {
    Ok(match kind {
        ScalarKind::I32 => {
            if is_ref {
                quote! { ::havok_native::hkx::types::HkxValue::I32(*#expr) }
            } else {
                quote! { ::havok_native::hkx::types::HkxValue::I32(#expr) }
            }
        }
        ScalarKind::U32 => {
            if is_ref {
                quote! { ::havok_native::hkx::types::HkxValue::U32(*#expr) }
            } else {
                quote! { ::havok_native::hkx::types::HkxValue::U32(#expr) }
            }
        }
        ScalarKind::F32 => {
            if is_ref {
                quote! { ::havok_native::hkx::types::HkxValue::F32(*#expr) }
            } else {
                quote! { ::havok_native::hkx::types::HkxValue::F32(#expr) }
            }
        }
        ScalarKind::Bool => {
            if is_ref {
                quote! { ::havok_native::hkx::types::HkxValue::Bool(*#expr) }
            } else {
                quote! { ::havok_native::hkx::types::HkxValue::Bool(#expr) }
            }
        }
        ScalarKind::StringVal => quote! {
            ::havok_native::hkx::types::HkxValue::String {
                value: #expr.to_string(),
                is_null: false,
            }
        },
    })
}

fn hkx_value_extract_scalar(
    kind: &ScalarKind,
    val_expr: TokenStream2,
) -> syn::Result<TokenStream2> {
    Ok(match kind {
        ScalarKind::I32 => quote! {
            match #val_expr {
                ::havok_native::hkx::types::HkxValue::I32(v) => ::std::result::Result::Ok(*v),
                other => ::std::result::Result::Err(format!("expected I32, got {:?}", other)),
            }
        },
        ScalarKind::U32 => quote! {
            match #val_expr {
                ::havok_native::hkx::types::HkxValue::U32(v) => ::std::result::Result::Ok(*v),
                other => ::std::result::Result::Err(format!("expected U32, got {:?}", other)),
            }
        },
        ScalarKind::F32 => quote! {
            match #val_expr {
                ::havok_native::hkx::types::HkxValue::F32(v) => ::std::result::Result::Ok(*v),
                other => ::std::result::Result::Err(format!("expected F32, got {:?}", other)),
            }
        },
        ScalarKind::Bool => quote! {
            match #val_expr {
                ::havok_native::hkx::types::HkxValue::Bool(v) => ::std::result::Result::Ok(*v),
                other => ::std::result::Result::Err(format!("expected Bool, got {:?}", other)),
            }
        },
        ScalarKind::StringVal => quote! {
            match #val_expr {
                ::havok_native::hkx::types::HkxValue::String { value, .. } => ::std::result::Result::Ok(value.clone()),
                other => ::std::result::Result::Err(format!("expected String, got {:?}", other)),
            }
        },
    })
}

// ─── Vec<T> inner type extraction ────────────────────────────────────────────

fn vec_inner_type(field: &syn::Field) -> syn::Result<ScalarKind> {
    let type_str = quote! { #field.ty }.to_string();
    let ty = &field.ty;
    let path = match ty {
        syn::Type::Path(tp) => &tp.path,
        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                "HkClass: #[hk_member(kind=\"array\")] requires a Vec<T> field",
            ));
        }
    };
    let last = path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(ty, "HkClass: empty type path"))?;
    if last.ident != "Vec" {
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "HkClass: #[hk_member(kind=\"array\")] requires Vec<T>, got '{}'",
                last.ident
            ),
        ));
    }
    let args = match &last.arguments {
        syn::PathArguments::AngleBracketed(ab) => &ab.args,
        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                "HkClass: Vec must have angle-bracketed type argument",
            ));
        }
    };
    let inner = args
        .first()
        .ok_or_else(|| syn::Error::new_spanned(ty, "HkClass: Vec must have one type argument"))?;
    let inner_ty = match inner {
        syn::GenericArgument::Type(t) => t,
        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                "HkClass: Vec type argument must be a type",
            ));
        }
    };
    // Suppress unused warning from type_str
    let _ = type_str;
    type_to_scalar_kind(inner_ty)
}

// ─── #[hk_class(...)] attribute parser ───────────────────────────────────────

fn parse_hk_class_attr(input: &DeriveInput) -> syn::Result<(String, u64)> {
    for attr in &input.attrs {
        if !attr.path().is_ident("hk_class") {
            continue;
        }
        let pairs: Punctuated<Meta, Token![,]> =
            attr.parse_args_with(Punctuated::parse_terminated)?;

        let mut name: Option<String> = None;
        let mut sig: Option<u64> = None;

        for meta in pairs {
            if let Meta::NameValue(nv) = meta {
                if nv.path.is_ident("name") {
                    if let Expr::Lit(el) = &nv.value {
                        if let Lit::Str(s) = &el.lit {
                            name = Some(s.value());
                        }
                    }
                } else if nv.path.is_ident("signature") {
                    if let Expr::Lit(el) = &nv.value {
                        if let Lit::Int(n) = &el.lit {
                            sig = Some(n.base10_parse::<u64>().unwrap_or(0));
                        }
                    }
                }
            }
        }

        let name = name.ok_or_else(|| {
            syn::Error::new_spanned(&input.ident, "#[hk_class] requires name = \"...\"")
        })?;
        let sig = sig.ok_or_else(|| {
            syn::Error::new_spanned(&input.ident, "#[hk_class] requires signature = 0x...")
        })?;
        return Ok((name, sig));
    }
    Err(syn::Error::new_spanned(
        &input.ident,
        "HkClass derive requires #[hk_class(name = \"...\", signature = 0x...)]",
    ))
}

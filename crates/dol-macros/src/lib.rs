#![forbid(unsafe_code)]
//! Procedural derive support for static DOL models.

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, parse_macro_input};

/// Derives the Phase-1 static model and record-view contracts.
///
/// Supported attributes:
/// - `#[dol(key = "stable.key", name = "CurrentName")]` on the struct.
/// - `#[dol(key = "stable.field", optional, identity, unique, with = path::Binding)]` on fields.
#[proc_macro_derive(Model, attributes(dol))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    match expand(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "generic models are not supported yet",
        ));
    }

    let mut model_key = ident.to_string();
    let mut model_name = ident.to_string();
    for attr in &input.attrs {
        if !attr.path().is_ident("dol") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                model_key = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            if meta.path.is_ident("name") {
                model_name = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            Err(meta.error("unsupported model-level `dol` attribute"))
        })?;
    }

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "Model can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "Model requires named fields",
        ));
    };

    let mut builder_steps = Vec::new();
    let mut field_consts = Vec::new();
    let mut access_fields = Vec::new();
    let mut mutate_fields = Vec::new();
    let mut identities = Vec::new();
    let mut uniques = Vec::new();

    for field in &fields.named {
        let Some(field_ident) = field.ident.as_ref() else {
            return Err(syn::Error::new_spanned(
                field,
                "Model requires named fields",
            ));
        };
        let field_ty = &field.ty;
        let field_name = field_ident.to_string();
        let mut field_key = field_name.clone();
        let mut optional = false;
        let mut identity = false;
        let mut unique = false;
        let mut binding = None::<syn::Path>;

        for attr in &field.attrs {
            if !attr.path().is_ident("dol") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("key") {
                    field_key = meta.value()?.parse::<LitStr>()?.value();
                    return Ok(());
                }
                if meta.path.is_ident("optional") {
                    optional = true;
                    return Ok(());
                }
                if meta.path.is_ident("identity") {
                    identity = true;
                    return Ok(());
                }
                if meta.path.is_ident("unique") {
                    unique = true;
                    return Ok(());
                }
                if meta.path.is_ident("with") {
                    binding = Some(meta.value()?.parse::<syn::Path>()?);
                    return Ok(());
                }
                Err(meta.error("unsupported field-level `dol` attribute"))
            })?;
        }

        let presence = if optional {
            quote!(::dol::core::model::Presence::Optional)
        } else {
            quote!(::dol::core::model::Presence::Required)
        };
        if let Some(binding) = binding.as_ref() {
            builder_steps.push(quote! {
                builder = builder.field_def(
                    #field_key,
                    #field_name,
                    <#binding as ::dol::core::binding::SemanticBinding<#field_ty>>::type_def(),
                    #presence,
                );
            });
            field_consts.push(quote! {
                #[allow(non_upper_case_globals)]
                pub const #field_ident: ::dol::core::model::Field<#field_ty> =
                    ::dol::core::model::Field::with_binding::<#binding>(
                        #model_key,
                        #field_key,
                        #field_name,
                    );
            });
            access_fields.push((
                field_key.clone(),
                quote! {
                    ::core::option::Option::Some(
                        <#binding as ::dol::core::binding::SemanticBinding<#field_ty>>::datum_ref(
                            &self.#field_ident
                        )
                    )
                },
            ));
            mutate_fields.push((
                field_key.clone(),
                quote! {
                    self.#field_ident =
                        <#binding as ::dol::core::binding::SemanticBinding<#field_ty>>::from_datum(
                            datum
                        )?;
                    ::core::result::Result::Ok(())
                },
            ));
        } else {
            builder_steps.push(quote! {
                builder = builder.field_def(
                    #field_key,
                    #field_name,
                    <#field_ty as ::dol::core::types::DataType>::type_def(),
                    #presence,
                );
            });
            field_consts.push(quote! {
                #[allow(non_upper_case_globals)]
                pub const #field_ident: ::dol::core::model::Field<#field_ty> =
                    ::dol::core::model::Field::with_key(#model_key, #field_key, #field_name);
            });
            access_fields.push((
                field_key.clone(),
                quote! {
                    ::core::option::Option::Some(
                        <#field_ty as ::dol::core::value::DataValue>::datum_ref(&self.#field_ident)
                    )
                },
            ));
            mutate_fields.push((
                field_key.clone(),
                quote! {
                    self.#field_ident =
                        <#field_ty as ::dol::core::value::DataValue>::from_datum(datum)?;
                    ::core::result::Result::Ok(())
                },
            ));
        }
        if identity {
            identities.push(field_key.clone());
        }
        if unique {
            uniques.push(field_key.clone());
        }
    }

    access_fields.sort_by(|left, right| left.0.cmp(&right.0));
    mutate_fields.sort_by(|left, right| left.0.cmp(&right.0));
    let access_arms = access_fields
        .iter()
        .enumerate()
        .map(|(index, (_, access))| quote!(#index => #access,));
    let mutate_arms = mutate_fields
        .iter()
        .enumerate()
        .map(|(index, (_, mutate))| quote!(#index => { #mutate },));
    let identity_step = if identities.is_empty() {
        quote!()
    } else {
        quote!(builder = builder.identity([#(#identities),*]);)
    };
    let unique_steps = uniques
        .iter()
        .map(|key| quote!(builder = builder.unique([#key]);));

    Ok(quote! {
        impl #ident {
            #(#field_consts)*

            #[doc(hidden)]
            fn __dol_model_def_ref() -> ::dol::core::diagnostic::Result<&'static ::dol::core::model::ModelDef> {
                static DEF: ::std::sync::OnceLock<
                    ::dol::core::diagnostic::Result<::dol::core::model::ModelDef>
                > = ::std::sync::OnceLock::new();
                let result = DEF.get_or_init(|| {
                    let mut builder =
                        ::dol::core::model::ModelBuilder::with_key(#model_key, #model_name);
                    #(#builder_steps)*
                    #identity_step
                    #(#unique_steps)*
                    builder.freeze()
                });
                match result {
                    ::core::result::Result::Ok(def) => ::core::result::Result::Ok(def),
                    ::core::result::Result::Err(error) => {
                        ::core::result::Result::Err(error.clone())
                    }
                }
            }
        }

        impl ::dol::core::model::Model for #ident {
            fn model_def() -> ::dol::core::diagnostic::Result<
                &'static ::dol::core::model::ModelDef
            > {
                Self::__dol_model_def_ref()
            }
        }

        impl ::dol::core::model::RecordView for #ident {
            fn model(
                &self,
            ) -> ::dol::core::diagnostic::Result<&::dol::core::model::ModelDef> {
                Self::__dol_model_def_ref()
            }

            fn field(
                &self,
                slot: ::dol::core::model::FieldSlot,
            ) -> ::dol::core::diagnostic::Result<
                ::core::option::Option<::dol::core::value::DatumRef<'_>>
            > {
                Self::__dol_model_def_ref()?;
                let datum = match slot.index() {
                    #(#access_arms)*
                    _ => ::core::option::Option::None,
                };
                ::core::result::Result::Ok(datum)
            }
        }

        impl ::dol::core::model::RecordMut for #ident {
            fn set_field(
                &mut self,
                slot: ::dol::core::model::FieldSlot,
                datum: ::dol::core::value::Datum,
            ) -> ::dol::core::diagnostic::Result<()> {
                let model = Self::__dol_model_def_ref()?;
                let field = model.fields().get(slot.index()).ok_or_else(|| {
                    ::dol::core::diagnostic::Diagnostic::error(
                        "RUNTIME-008",
                        "field slot is outside the static model",
                    )
                })?;
                ::dol::core::value::validate_datum(field.ty(), field.presence(), &datum)?;
                match slot.index() {
                    #(#mutate_arms)*
                    _ => ::core::result::Result::Err(
                        ::dol::core::diagnostic::Diagnostic::error(
                            "RUNTIME-008",
                            "field slot is outside the static record",
                        ),
                    ),
                }
            }
        }
    })
}

/// Derives a typed named-record result usable with `Pipeline::select`.
///
/// Supported attributes:
/// - `#[dol(key = "stable/type-key", version = 1)]` on the struct.
/// - `#[dol(name = "logical_name", with = path::Binding)]` on fields.
#[proc_macro_derive(Projection, attributes(dol))]
pub fn derive_projection(input: TokenStream) -> TokenStream {
    match expand_projection(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_projection(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "generic projection records are not supported yet",
        ));
    }

    let mut type_key = format!("projection/{ident}");
    let mut version = 1_u32;
    for attr in &input.attrs {
        if !attr.path().is_ident("dol") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                type_key = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            if meta.path.is_ident("version") {
                version = meta.value()?.parse::<syn::LitInt>()?.base10_parse()?;
                return Ok(());
            }
            Err(meta.error("unsupported projection-level `dol` attribute"))
        })?;
    }
    if version == 0 {
        return Err(syn::Error::new_spanned(
            ident,
            "projection semantic version must be greater than zero",
        ));
    }

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "Projection can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "Projection requires named fields",
        ));
    };
    if fields.named.is_empty() {
        return Err(syn::Error::new_spanned(
            ident,
            "Projection requires at least one field",
        ));
    }
    if fields.named.len() > 12 {
        return Err(syn::Error::new_spanned(
            fields,
            "Projection currently supports at most 12 fields",
        ));
    }

    let mut type_fields = Vec::new();
    let mut projection_fields = Vec::new();
    let mut field_types = Vec::new();
    let mut value_fields = Vec::new();

    for field in &fields.named {
        let Some(field_ident) = field.ident.as_ref() else {
            return Err(syn::Error::new_spanned(
                field,
                "Projection requires named fields",
            ));
        };
        let field_ty = &field.ty;
        let mut field_name = field_ident.to_string();
        let mut binding = None::<syn::Path>;

        for attr in &field.attrs {
            if !attr.path().is_ident("dol") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    field_name = meta.value()?.parse::<LitStr>()?.value();
                    return Ok(());
                }
                if meta.path.is_ident("with") {
                    binding = Some(meta.value()?.parse::<syn::Path>()?);
                    return Ok(());
                }
                Err(meta.error("unsupported projection-field `dol` attribute"))
            })?;
        }

        let type_expr = if let Some(binding) = binding.as_ref() {
            quote!(<#binding as ::dol::core::binding::SemanticBinding<#field_ty>>::type_def())
        } else {
            quote!(<#field_ty as ::dol::core::types::DataType>::type_def())
        };
        let datum_expr = if let Some(binding) = binding.as_ref() {
            quote!(
                <#binding as ::dol::core::binding::SemanticBinding<#field_ty>>::datum_ref(
                    &self.#field_ident
                )
            )
        } else {
            quote!(<#field_ty as ::dol::core::value::DataValue>::datum_ref(&self.#field_ident))
        };

        type_fields.push(quote! {
            ::dol::core::types::RecordField::required(#field_name, #type_expr)
        });
        projection_fields.push(quote! {
            ::dol::core::types::RecordField::required(#field_name, #type_expr)
        });
        field_types.push(quote!(#field_ty));
        value_fields.push((field_name, datum_expr));
    }

    let field_count = fields.named.len();
    value_fields.sort_by(|left, right| left.0.cmp(&right.0));
    let value_arms = value_fields.iter().enumerate().map(
        |(index, (name, datum))| quote!(#index => ::core::option::Option::Some((#name, #datum)),),
    );

    Ok(quote! {
        impl ::dol::core::types::DataType for #ident {
            fn type_def() -> ::dol::core::types::TypeDef {
                ::dol::core::types::TypeDef::shaped(
                    #type_key,
                    #version,
                    ::dol::core::types::TypeShape::Record(vec![#(#type_fields),*]),
                )
            }
        }

        impl ::dol::core::value::RecordValueView for #ident {
            fn len(&self) -> usize {
                #field_count
            }

            fn field(
                &self,
                index: usize,
            ) -> ::core::option::Option<(&str, ::dol::core::value::DatumRef<'_>)> {
                match index {
                    #(#value_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }

        impl ::dol::core::value::DataValue for #ident {
            fn datum_ref(&self) -> ::dol::core::value::DatumRef<'_> {
                ::dol::core::value::DatumRef::Value(
                    ::dol::core::value::ValueRef::Record(self)
                )
            }
        }

        impl ::dol::core::pipeline::ProjectionRecord for #ident {
            type Fields = (#(#field_types,)*);

            fn __projection_fields() -> ::std::vec::Vec<::dol::core::types::RecordField> {
                vec![#(#projection_fields),*]
            }
        }

        impl #ident {
            /// Builds a typed named-record projection from declaration-order symbolic sources.
            #[must_use]
            pub fn project<P>(sources: P) -> ::dol::core::pipeline::RecordProjection<Self>
            where
                P: ::dol::core::pipeline::ProjectionTuple<
                    Value = <Self as ::dol::core::pipeline::ProjectionRecord>::Fields,
                >,
            {
                ::dol::core::pipeline::RecordProjection::<Self>::new(sources)
            }
        }
    })
}

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields};

pub fn expand(item: TokenStream) -> TokenStream {
    let input = match syn::parse2::<DeriveInput>(item) {
        Ok(input) => input,
        Err(error) => return error.to_compile_error(),
    };
    if !input.generics.params.is_empty() {
        return Error::new_spanned(&input.generics, "BcsSchema cannot be derived for generic types")
            .to_compile_error();
    }
    let ident = &input.ident;
    let name = ident.to_string();
    let schema = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => {
                let entries = fields.named.iter().map(|field| {
                    let field_name = field.ident.as_ref().unwrap().to_string();
                    let ty = &field.ty;
                    quote!((#field_name.to_string(), <#ty as ::ech_db_entity::BcsSchema>::schema()))
                });
                quote! {
                    ::ech_db_entity::Schema::Struct {
                        name: #name.to_string(),
                        fields: vec![ #(#entries),* ],
                    }
                }
            }
            _ => {
                return Error::new_spanned(&data.fields, "BcsSchema needs a struct with named fields")
                    .to_compile_error()
            }
        },
        Data::Enum(data) => {
            let variants = data.variants.iter().map(|variant| {
                let variant_name = variant.ident.to_string();
                match &variant.fields {
                    Fields::Unit => {
                        quote!((#variant_name.to_string(), ::ech_db_entity::VariantSchema::Unit))
                    }
                    Fields::Named(fields) => {
                        let entries = fields.named.iter().map(|field| {
                            let field_name = field.ident.as_ref().unwrap().to_string();
                            let ty = &field.ty;
                            quote!((#field_name.to_string(), <#ty as ::ech_db_entity::BcsSchema>::schema()))
                        });
                        quote! {
                            (
                                #variant_name.to_string(),
                                ::ech_db_entity::VariantSchema::Fields(vec![ #(#entries),* ]),
                            )
                        }
                    }
                    Fields::Unnamed(_) => {
                        return Error::new_spanned(
                            variant,
                            "BcsSchema needs named or unit enum variants",
                        )
                        .to_compile_error()
                    }
                }
            });
            quote! {
                ::ech_db_entity::Schema::Enum {
                    name: #name.to_string(),
                    variants: vec![ #(#variants),* ],
                }
            }
        }
        Data::Union(_) => {
            return Error::new_spanned(&input, "BcsSchema cannot be derived for unions")
                .to_compile_error()
        }
    };
    quote! {
        impl ::ech_db_entity::BcsSchema for #ident {
            fn schema() -> ::ech_db_entity::Schema {
                #schema
            }
        }
    }
}

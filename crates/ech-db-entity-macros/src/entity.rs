use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parenthesized, Error, Ident, ItemEnum, Path, Token};

use crate::model::{parse_entity_item, EntityModel};

pub struct EntityAttr {
    event: Path,
    key: Vec<Ident>,
}

impl Parse for EntityAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut event = None;
        let mut key = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if name == "event" {
                if event.is_some() {
                    return Err(Error::new_spanned(name, "duplicate event"));
                }
                event = Some(input.parse::<Path>()?);
            } else if name == "key" {
                if key.is_some() {
                    return Err(Error::new_spanned(name, "duplicate key"));
                }
                let content;
                parenthesized!(content in input);
                let idents = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?;
                key = Some(idents.into_iter().collect());
            } else {
                return Err(Error::new_spanned(name, "expected event = ... or key = (...)"));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Self {
            event: event.ok_or_else(|| Error::new(input.span(), "event = <path> is required"))?,
            key: key.ok_or_else(|| Error::new(input.span(), "key = (...) is required"))?,
        })
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = match syn::parse2::<EntityAttr>(attr) {
        Ok(attr) => attr,
        Err(error) => return error.to_compile_error(),
    };
    let item_enum = match syn::parse2::<ItemEnum>(item) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error(),
    };
    let mut item_enum = item_enum;
    let model = match parse_entity_item(&mut item_enum, attr.event, attr.key) {
        Ok(model) => model,
        Err(error) => return error.to_compile_error(),
    };
    let meta = meta_impl(&model);
    let inherent = inherent_impl(&model);
    quote! {
        #item_enum
        #meta
        #inherent
    }
}

fn meta_impl(model: &EntityModel) -> TokenStream {
    let ident = &model.ident;
    let event = &model.event;
    let type_name = ident.to_string();
    let latest = model.versions.len() as u32;
    let key_idents = &model.key;

    let version_arms = model.versions.iter().map(|version| {
        let variant = &version.ident;
        let number = version.version;
        quote!(#ident::#variant { .. } => #number)
    });

    let key_arms = model.versions.iter().map(|version| {
        let variant = &version.ident;
        quote! {
            #ident::#variant { #(#key_idents,)* .. } => {
                let key = (#(#key_idents,)*);
                ::ech_db_entity::bcs::to_bytes(&key).map_err(|_| ::ech_db_entity::Error::Encode)
            }
        }
    });

    let visit_arms = model.versions.iter().map(|version| {
        let variant = &version.ident;
        let collections: Vec<&Ident> = version
            .fields
            .iter()
            .filter(|field| field.kind.is_collection())
            .map(|field| &field.ident)
            .collect();
        let idents_pattern: Vec<&Ident> = collections.clone();
        let idents_body: Vec<&Ident> = collections.clone();
        let names: Vec<String> = collections.iter().map(|ident| ident.to_string()).collect();
        quote! {
            #ident::#variant { #(#idents_pattern,)* .. } => {
                #( visitor(#names, #idents_body)?; )*
            }
        }
    });

    let transitions = model.transitions.iter().map(|transition| {
        let from = transition.from;
        let old = &model.versions[(from - 1) as usize];
        let new = &model.versions[from as usize];
        let old_variant = &old.ident;
        let new_variant = &new.ident;
        let patterns = &transition.old_patterns;
        let values = &transition.new_values;
        let assignments = new.fields.iter().zip(values.iter()).map(|(field, value)| {
            let name = &field.ident;
            quote!(#name: #value)
        });
        quote! {
            #from => {
                let old = ctx.take(state)?;
                let #ident::#old_variant { #(#patterns),* } = old else {
                    return Err(::ech_db_entity::Error::MigrationInvalid);
                };
                ctx.install(state, #ident::#new_variant { #(#assignments),* })?;
                Ok(())
            }
        }
    });

    let schema_versions = model.versions.iter().map(|version| {
        let number = version.version;
        let migration = if version.version == 1 {
            quote!(::ech_db_entity::MigrationKind::Initial)
        } else {
            quote!(::ech_db_entity::MigrationKind::Auto)
        };
        let fields = version.fields.iter().map(|field| {
            let name = field.ident.to_string();
            let kind = field.kind.meta_variant();
            let ty = &field.ty_key;
            let ty_tokens = &field.ty;
            quote! {
                ::ech_db_entity::FieldSchema {
                    name: #name.to_string(),
                    kind: #kind,
                    ty: #ty.to_string(),
                    bcs: <#ty_tokens as ::ech_db_entity::BcsSchema>::schema(),
                }
            }
        });
        quote! {
            ::ech_db_entity::VersionSchema {
                version: #number,
                migration: #migration,
                fields: vec![ #(#fields),* ],
            }
        }
    });

    let key_strings: Vec<String> = model.key.iter().map(|ident| ident.to_string()).collect();

    quote! {
        impl ::ech_db_entity::EntityMeta for #ident {
            type Event = #event;

            const TYPE_NAME: &'static str = #type_name;
            const LATEST_VERSION: u32 = #latest;

            fn version(&self) -> u32 {
                match self {
                    #( #version_arms, )*
                }
            }

            fn key_bytes(&self) -> ::ech_db_entity::Result<Vec<u8>> {
                match self {
                    #( #key_arms, )*
                }
            }

            fn is_genesis(event: &Self::Event) -> bool {
                matches!(event, #event::Genesis { .. })
            }

            fn visit_collections(
                &mut self,
                visitor: &mut dyn FnMut(&'static str, &mut dyn ::ech_db_entity::CollectionSlot) -> ::ech_db_entity::Result<()>,
            ) -> ::ech_db_entity::Result<()> {
                match self {
                    #( #visit_arms, )*
                }
                Ok(())
            }

            fn migrate_step(
                state: &mut Self,
                from: u32,
                ctx: &mut ::ech_db_entity::EntityContext,
            ) -> ::ech_db_entity::Result<()> {
                match from {
                    #( #transitions, )*
                    _ => Err(::ech_db_entity::Error::MigrationInvalid),
                }
            }

            fn schema() -> ::ech_db_entity::EntitySchema {
                ::ech_db_entity::EntitySchema {
                    name: #type_name.to_string(),
                    key: vec![ #( #key_strings.to_string(), )* ],
                    versions: vec![ #( #schema_versions, )* ],
                }
            }
        }
    }
}

fn inherent_impl(model: &EntityModel) -> TokenStream {
    let ident = &model.ident;
    let accessors = model.accessors.iter().map(|accessor| {
        let name = &accessor.name;
        let mut_name = &accessor.mut_name;
        let ty = &accessor.ty;
        let arms: Vec<TokenStream> = accessor
            .arms
            .iter()
            .map(|(variant, field)| quote!(#ident::#variant { #field, .. } => Ok(#field)))
            .collect();
        let fallback = if accessor.covered_all {
            quote!()
        } else {
            quote!(_ => Err(::ech_db_entity::Error::UnsupportedVersion(<Self as ::ech_db_entity::EntityMeta>::version(self))),)
        };
        let deprecated = if accessor.deprecated {
            quote!(#[deprecated])
        } else {
            quote!()
        };
        quote! {
            #deprecated
            pub fn #name(&self) -> ::ech_db_entity::Result<&#ty> {
                match self {
                    #( #arms, )*
                    #fallback
                }
            }

            #deprecated
            fn #mut_name(&mut self) -> ::ech_db_entity::Result<&mut #ty> {
                match self {
                    #( #arms, )*
                    #fallback
                }
            }
        }
    });
    quote! {
        impl #ident where #ident: ::ech_db_entity::EntityMeta {
            pub fn load(id: ::ech_db_entity::EntityId) -> ::ech_db_entity::Result<::ech_db_entity::Guard<Self>> {
                ::ech_db_entity::runtime::load::<Self>(id)
            }

            pub fn load_root() -> ::ech_db_entity::Result<::ech_db_entity::Guard<Self>> {
                ::ech_db_entity::runtime::load_root::<Self>()
            }

            pub fn id(&self) -> ::ech_db_entity::Result<::ech_db_entity::EntityId> {
                ::ech_db_entity::runtime::attached_id(self)
            }

            pub fn spawn<E: ::ech_db_entity::Entity>(
                &mut self,
                event: &E::Event,
            ) -> ::ech_db_entity::Result<::ech_db_entity::Guard<E>> {
                let parent = ::ech_db_entity::runtime::attached_id(self)?;
                ::ech_db_entity::runtime::spawn_child::<E>(parent, event)
            }

            pub fn upgrade(&mut self) -> ::ech_db_entity::Result<()> {
                ::ech_db_entity::runtime::upgrade(self)
            }

            pub fn events(&self) -> ::ech_db_entity::Result<::ech_db_entity::EventLog<<Self as ::ech_db_entity::EntityMeta>::Event>> {
                ::ech_db_entity::runtime::events(self)
            }

            #( #accessors )*
        }
    }
}

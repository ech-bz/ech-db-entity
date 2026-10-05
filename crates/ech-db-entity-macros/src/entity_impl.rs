use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, FnArg, Ident, ImplItem, ItemImpl, Pat, ReturnType, Type};

pub fn expand(item: TokenStream) -> TokenStream {
    let mut item_impl = match syn::parse2::<ItemImpl>(item) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error(),
    };
    if !item_impl.generics.params.is_empty() || item_impl.generics.where_clause.is_some() {
        return Error::new_spanned(&item_impl.generics, "entity impl cannot be generic")
            .to_compile_error();
    }
    let self_ty = item_impl.self_ty.clone();
    let mut genesis = None;
    let mut apply = None;
    for entry in &item_impl.items {
        if let ImplItem::Fn(function) = entry {
            if function.sig.ident == "genesis" {
                if genesis.is_some() {
                    return Error::new_spanned(&function.sig, "duplicate genesis").to_compile_error();
                }
                genesis = Some(function.clone());
            } else if function.sig.ident == "apply" {
                if apply.is_some() {
                    return Error::new_spanned(&function.sig, "duplicate apply").to_compile_error();
                }
                apply = Some(function.clone());
            }
        }
    }
    let Some(genesis) = genesis else {
        return Error::new_spanned(&item_impl, "entity impl needs fn genesis").to_compile_error();
    };
    let Some(apply) = apply else {
        return Error::new_spanned(&item_impl, "entity impl needs fn apply").to_compile_error();
    };
    let (event_ident, apply_signature, apply_attrs) = match validate_apply(&apply) {
        Ok(parts) => parts,
        Err(error) => return error.to_compile_error(),
    };
    let genesis_ident = Ident::new("__genesis_impl", genesis.sig.ident.span());
    let body_ident = Ident::new("__apply_body", apply.sig.ident.span());
    for entry in &mut item_impl.items {
        if let ImplItem::Fn(function) = entry {
            if function.sig.ident == "genesis" {
                function.sig.ident = genesis_ident.clone();
            } else if function.sig.ident == "apply" {
                function.sig.ident = body_ident.clone();
            }
        }
    }
    let inputs = &apply_signature.inputs;
    let output = &apply_signature.output;
    let wrapper = quote! {
        #(#apply_attrs)*
        pub fn apply(#inputs) #output {
            ::ech_db_entity::runtime::apply::<Self>(self, #event_ident)
        }
    };
    let self_ty = &self_ty;
    quote! {
        #item_impl
        impl #self_ty {
            #wrapper
        }
        impl ::ech_db_entity::EntityHandlers for #self_ty {
            fn genesis(event: &<Self as ::ech_db_entity::EntityMeta>::Event) -> ::ech_db_entity::Result<Self> {
                <Self>::__genesis_impl(event)
            }

            fn body(&mut self, event: &<Self as ::ech_db_entity::EntityMeta>::Event) -> ::ech_db_entity::Result<()> {
                <Self>::__apply_body(self, event)
            }
        }
    }
}

fn validate_apply(
    function: &syn::ImplItemFn,
) -> Result<(Ident, syn::Signature, Vec<syn::Attribute>), Error> {
    let mut inputs = function.sig.inputs.iter();
    match inputs.next() {
        Some(FnArg::Receiver(receiver)) if receiver.reference.is_some() && receiver.mutability.is_some() => {}
        _ => {
            return Err(Error::new_spanned(
                &function.sig,
                "apply must take &mut self",
            ))
        }
    }
    let event_ident = match inputs.next() {
        Some(FnArg::Typed(argument)) => match argument.pat.as_ref() {
            Pat::Ident(pat) => pat.ident.clone(),
            _ => {
                return Err(Error::new_spanned(
                    &argument.pat,
                    "apply event parameter must be a plain identifier",
                ))
            }
        },
        _ => return Err(Error::new_spanned(&function.sig, "apply must take an event")),
    };
    if inputs.next().is_some() {
        return Err(Error::new_spanned(
            &function.sig,
            "apply must take exactly one event parameter",
        ));
    }
    if let FnArg::Typed(argument) = &function.sig.inputs[1] {
        if !matches!(&*argument.ty, Type::Reference(reference) if reference.mutability.is_none()) {
            return Err(Error::new_spanned(
                &argument.ty,
                "apply event parameter must be a shared reference",
            ));
        }
    }
    if let ReturnType::Default = function.sig.output {
        return Err(Error::new_spanned(
            &function.sig.output,
            "apply must return Result<()>",
        ));
    }
    Ok((
        event_ident,
        function.sig.clone(),
        function.attrs.clone(),
    ))
}

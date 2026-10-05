mod bcs_schema;
mod entity;
mod entity_impl;
mod model;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn entity(attr: TokenStream, item: TokenStream) -> TokenStream {
    entity::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn entity_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    entity_impl::expand(item.into()).into()
}

#[proc_macro_derive(BcsSchema)]
pub fn bcs_schema(item: TokenStream) -> TokenStream {
    bcs_schema::expand(item.into()).into()
}

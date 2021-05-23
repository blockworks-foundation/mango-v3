use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(Loadable)]
pub fn loadable(input: TokenStream) -> TokenStream {

    let DeriveInput { ident, data, .. }  = parse_macro_input!(input);

    match data {
        syn::Data::Struct(_) => {
            quote! {
                impl mango_common::Loadable for #ident {}
            }
        }

        _ => panic!()
    }.into()
}

/// This must be derived first before Loadable can be derived
#[proc_macro_derive(Pod)]
pub fn pod(input: TokenStream) -> TokenStream {

    let DeriveInput {ident, data, .. } = parse_macro_input!(input);

    match data {
        syn::Data::Struct(_) => {
            quote! {

                unsafe impl bytemuck::Zeroable for #ident {}
                unsafe impl bytemuck::Pod for #ident {}

            }
        }

        _ => panic!()
    }.into()
}

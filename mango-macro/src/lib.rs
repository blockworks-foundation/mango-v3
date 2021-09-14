use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derive `Pod` first before deriving this
#[proc_macro_derive(Loadable)]
pub fn loadable(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    match data {
        syn::Data::Struct(_) => {
            quote! {
                impl mango_common::Loadable for #ident {}
            }
        }

        _ => panic!(),
    }
    .into()
}

/// This must be derived first before Loadable can be derived
#[proc_macro_derive(Pod)]
pub fn pod(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    match data {
        syn::Data::Struct(_) => {
            quote! {
                unsafe impl bytemuck::Zeroable for #ident {}
                unsafe impl bytemuck::Pod for #ident {}
            }
        }

        _ => panic!(),
    }
    .into()
}

/// Makes a struct trivially transmutable i.e. safe to read and write into an arbitrary slice of bytes
#[proc_macro_derive(TriviallyTransmutable)]
pub fn trivially_transmutable(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    match data {
        syn::Data::Struct(_) => {
            quote! {
                unsafe impl safe_transmute::trivial::TriviallyTransmutable for #ident {}
            }
        }

        _ => panic!(),
    }
    .into()
}

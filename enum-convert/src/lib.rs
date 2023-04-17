use proc_macro::{self, TokenStream};
use quote::quote;
use syn::parse_macro_input;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Skip,
    Do,
}

#[proc_macro_derive(EnumConvert, attributes(enum_convert))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemEnum);
    let enum_ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let mut output = proc_macro2::TokenStream::new();

    let outer_attr = input
        .attrs
        .iter()
        .find(|v| v.path().is_ident("enum_convert"));
    let mut into_mode: Mode = Mode::Skip;
    let mut from_mode: Mode = Mode::Skip;
    if let Some(attr) = outer_attr {
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("into") {
                into_mode = Mode::Do;
            } else if meta.path.is_ident("from") {
                from_mode = Mode::Do;
            }
            Ok(())
        });
    }

    for variant in input.variants {
        let variant_ident = variant.ident;
        let type_path = match variant.fields {
            syn::Fields::Unnamed(v) => match v.unnamed.first() {
                Some(v) => match &v.ty {
                    syn::Type::Path(v) => v.to_owned(),
                    _ => continue,
                },
                None => continue,
            },
            _ => continue,
        };

        let mut into_override = into_mode;
        let mut from_override = from_mode;
        let inner_attr = variant
            .attrs
            .iter()
            .find(|v| v.path().is_ident("enum_convert"));
        if let Some(attr) = inner_attr {
            into_override = Mode::Skip;
            from_override = Mode::Skip;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                } else if meta.path.is_ident("from") {
                    from_override = Mode::Do;
                } else if meta.path.is_ident("into") {
                    into_override = Mode::Do;
                } else {
                    return Err(meta.error("Invalid mode override"));
                }
                Ok(())
            })
            .unwrap()
        }
        println!("{:?} into, {:?} from", into_override, from_override);

        if let Mode::Do = from_override {
            output.extend(quote! {
                impl #impl_generics From<#type_path> for #enum_ident #ty_generics #where_clause {
                    fn from(value: #type_path) -> Self {
                        Self::#variant_ident(value)
                    }
                }
            })
        }
        if let Mode::Do = into_override {
            output.extend(quote! {
                impl #impl_generics TryInto<#type_path> for #enum_ident #ty_generics #where_clause {
                    type Error = String;
                    fn try_into(self) -> Result<#type_path, Self::Error> {
                        match self {
                            Self::#variant_ident(value) => Ok(value),
                            _ => Err("Incorrect Variant".to_string()),
                        }
                    }
                }
            });
        }
    }
    output.into()
}

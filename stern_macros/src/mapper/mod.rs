use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, Type, TypePath, parse_quote};

use crate::{DefineMapperInput, mapper_trait_ident, mapper_trait_member_type_ident};

pub mod parse;

pub fn generate(input: DefineMapperInput) -> TokenStream {
    let mapper_trait_name = mapper_trait_ident(&input.base_name.name);

    let impl_items = input.properties.properties.iter().map(|prop| {
        let type_member_name = mapper_trait_member_type_ident(&prop.name);

        let slint_typ = map_slint_private_type(&prop.slint_typ.typ);

        let domain_typ = prop
            .domain_typ
            .as_ref()
            .map(|v| &v.typ)
            .unwrap_or(&slint_typ);

        let default_mapper: Expr = parse_quote! {
            |value: #domain_typ| value
        };
        let mapper_expr = prop
            .mapper
            .as_ref()
            .map(|v| &v.expr)
            .unwrap_or(&default_mapper);

        let map_method_name = format_ident!("map_{}", &prop.name);
        quote! {
            type #type_member_name = #domain_typ;
            fn #map_method_name(value: Self::#type_member_name) -> #slint_typ {
                (#mapper_expr)(value)
            }
        }
    });
    quote! {
        #[derive(Debug, Clone)]
        pub struct Mapper;

        impl #mapper_trait_name for Mapper {
            #(#impl_items)*
        }
    }
}

/// Maps `sp::Xxx` to `slint::Xxx`.
fn map_slint_private_type(typ: &Type) -> Type {
    match typ {
        Type::Path(TypePath {
            attrs: _,
            qself: _,
            path,
        }) => {
            let mut segs = path.segments.iter();
            let first = segs.next();
            if first.is_none() || first.is_some_and(|v| v.ident != "sp") {
                return typ.clone();
            };
            let secound = segs.next();
            match secound {
                Some(v) => match v.ident.to_string().as_str() {
                    "SharedString" => parse_quote!(slint::SharedString),
                    _ => typ.clone(),
                },
                _ => typ.clone(),
            }
        }
        _ => typ.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use crate::{PropertyField, generate::generate_mapper_trait};

    use super::*;

    #[test]
    fn generate_output_is_correct() {
        let input: DefineMapperInput = syn::parse_str(
            "base_name: Score,
            properties: {
                score: {
                    domain_typ: u32,
                    slint_typ: i32,
                    mapper: {
                        |value:u32| value.try_into().expect(\"failed to convert u32 into i32\")
                    },
                },
            }",
        )
        .unwrap();
        let output = generate(input);
        let syn_file = syn::parse_file(output.to_string().as_str()).unwrap();
        let pretty = prettyplease::unparse(&syn_file);
        insta::assert_snapshot!(pretty);

        /*let mut file = new_trybuild_file!();
        file.write_all(pretty.as_bytes()).unwrap();
        file.write_all("fn main(){}".as_bytes()).unwrap();

        {
            let mapper_trait = generate_mapper_trait(
                &format_ident!("Score"),
                &vec![PropertyField {
                    ident: format_ident!("score"),
                    ty: parse_quote!(i32),
                }],
            );
            let syn_file = syn::parse_file(mapper_trait.to_string().as_str()).unwrap();
            let pretty = prettyplease::unparse(&syn_file);
            file.write_all(pretty.as_bytes()).unwrap();
        }
        let t = trybuild::TestCases::new();
        t.pass(file.path());*/
    }

    #[test]
    fn map_slint_private_type_maps_shared_string() {
        let priv_typ = parse_quote!(sp::SharedString);
        let mapped = map_slint_private_type(&priv_typ);
        assert_eq!(mapped, parse_quote!(slint::SharedString));
    }
}

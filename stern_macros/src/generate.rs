use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type, ext::IdentExt};

use crate::{
    InnerGlobalComponent, PropertyField, define_mapper_macro_ident, mapper_trait_ident,
    mapper_trait_member_type_ident, public_global_ident, state_struct_ident, viewmodel_trait_ident,
};

pub fn adopter_inner(_attr: TokenStream, item: InnerGlobalComponent) -> TokenStream {
    let viewmodel_trait_name = viewmodel_trait_ident(&item.name);

    let state_struct = generate_state_struct(&item.name, &item.properties);
    let mapper_trait = generate_mapper_trait(&item.name, &item.properties);
    let mapper_macro = generate_define_mapper_macro(&item.name, &item.properties);
    let viewmodel_trait = {
        let callbacks = item.callbacks.iter().map(|f| {
            let fn_name = format_ident!("on_{}", &f.ident);
            let args = f.arg_type.elems.iter().enumerate().map(|(idx, typ)| {
                let arg_name = format_ident!("args_{}", idx);
                quote! {
                    #arg_name: #typ,
                }
            });
            let ret_typ = &f.ret_type;
            quote! {
                fn #fn_name(&mut self, #(#args)*) -> #ret_typ;
            }
        });

        quote! {
            pub trait #viewmodel_trait_name {
                #(#callbacks)*
            }
        }
    };
    let regiter_impl = {
        let public_global_name = public_global_ident(&item.name);

        let registerations = item.callbacks.iter().map(|cb| {
            let method_name = format_ident!("on_{}", cb.ident);
            let args = cb
                .arg_type
                .elems
                .iter()
                .enumerate()
                .map(|(idx, _ty)| format_ident!("arg_{}", idx));
            let args = quote! {
                #(#args),*
            };
            quote! {
                self.#method_name({
                    let vm_clone = std::rc::Rc::clone(&vm);
                    move |#args| {
                        vm_clone.borrow_mut().#method_name(#args)
                    }
                });
            }
        });

        quote! {
            impl<VMImpl> stern::GlobalExt<VMImpl> for #public_global_name<'_>
                where VMImpl: #viewmodel_trait_name + 'static {
                fn register_viewmodel(&self, vm: std::rc::Rc<core::cell::RefCell<VMImpl>>){
                    #(#registerations)*
                }
            }
        }
    };
    let original = item.original;
    quote! {
        #original

        #state_struct

        #viewmodel_trait

        #mapper_trait

        #mapper_macro

        #regiter_impl
    }
}

pub(crate) fn generate_state_struct(base: &Ident, properties: &Vec<PropertyField>) -> TokenStream {
    let state_struct_name = state_struct_ident(base);
    let public_global_name = public_global_ident(base);
    let mapper_trait_name = mapper_trait_ident(base);

    let state_struct_def = {
        let fields = properties.iter().map(|f| {
            let field_name = &f.ident;
            let field_ty = &f.ty;
            let trait_type_member = mapper_trait_member_type_ident(&f.ident);
            quote! {
                pub #field_name: stern::MappedPropertyHandle<M::#trait_type_member, #field_ty>,
            }
        });

        quote! {
            #[derive(Debug)]
            pub struct #state_struct_name<M> where M: #mapper_trait_name{
                #(#fields)*
                _phantom: std::marker::PhantomData<M>, // fields.len() == 0のときM is never usedにならないように
                // TODO: fields.len() == 0のときstructを生成しないほうが良いかも？
            }
        }
    };
    let state_struct_impl = {
        let field_impls = properties.iter().map(|f| {
            let field_name = &f.ident;
            let set_method_name = format_ident!("set_{}", &f.ident);
            let map_method_name = format_ident!("map_{}", &f.ident);
            quote! {
                #field_name: stern::MappedPropertyHandle::new_mapped(
                    {
                        // setter
                        let adopter_weak = adopter_weak.clone();
                        move |val| {
                            let adopter_strong = adopter_weak.unwrap();
                            adopter_strong.#set_method_name(val)
                        }
                    },
                    {
                        // mapper
                        move |val| {
                            M::#map_method_name(val)
                        }
                    },
                ),
            }
        });
        quote! {
            impl<M> #state_struct_name<M>
                where M: Clone + #mapper_trait_name {
                pub fn new(adopter_weak: slint::Weak<#public_global_name<'static>>) -> Self {
                    Self {
                        #(#field_impls)*
                        _phantom: std::marker::PhantomData,
                    }
                }
            }
        }
    };
    quote! {
        #state_struct_def

        #state_struct_impl
    }
}

pub(crate) fn generate_mapper_trait(base: &Ident, properties: &Vec<PropertyField>) -> TokenStream {
    let trait_name = mapper_trait_ident(base);
    let members = properties.iter().map(|f| {
        let type_member_name = mapper_trait_member_type_ident(&f.ident);
        let fn_name = format_ident!("map_{}", f.ident);

        let default_type = if let Type::Path(p) = &f.ty {
            &p.path
        } else {
            panic!("this kind of type is not supported as property type");
        };
        /* = #default_type; // associated type defaults are unstable*/
        quote! {
            type #type_member_name: Default;
            fn #fn_name(value: Self::#type_member_name) -> #default_type;
        }
    });
    quote! {
        pub trait #trait_name {
            #(#members)*
        }
    }
}

pub(crate) fn generate_define_mapper_macro(
    base_name: &Ident,
    properties: &Vec<PropertyField>,
) -> TokenStream {
    let macro_name = define_mapper_macro_ident(base_name);
    let matchers = properties.iter().map(|f| {
        let name = &f.ident.unraw();
        let typ_ident = format_ident!("{}_typ", f.ident);
        let mapper_ident = format_ident!("{}_mapper", f.ident);
        quote! {
            $(
                #name to $#typ_ident:ty {
                    $#mapper_ident:expr
                },
            )?
        }
    });
    let impl_calls = properties.iter().map(|f| {
        let name = &f.ident;

        let slint_typ_original = &f.ty;
        let slint_typ = if let Type::Path(tp) = &f.ty {
            let ident = tp.path.segments.first().unwrap().ident.clone();
            let unrawed = ident.unraw();
            if ident != unrawed {
                // slint_typ marked with 'r#' is always slint-generated type (enum or struct in *.slint file).
                // parse_quote!(concat!module_path!(), #unrawed))
                quote!($crate::#unrawed)
            } else {
                quote! {#slint_typ_original}
            }
        } else {
            quote! {#slint_typ_original}
        };

        let typ_ident = format_ident!("{}_typ", f.ident);
        let mapper_ident = format_ident!("{}_mapper", f.ident);

        quote! {
            #name: {
                $(domain_typ: $#typ_ident,)?
                slint_typ: #slint_typ,
                $(mapper: $#mapper_ident,)?
            },
        }
    });
    quote! {
        #[macro_export]
        macro_rules! #macro_name {
            {
                #(#matchers)*
            } => {
                stern::define_mapper_impl!{
                    base_name: #base_name,
                    properties: {
                        #(#impl_calls)*
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use syn::parse_quote;

    use super::*;

    #[test]
    fn generate_define_mapper_macro_snapshot() {
        let properties = vec![PropertyField {
            ident: format_ident!("score"),
            ty: syn::parse_quote!(i32),
        }];
        let output = generate_define_mapper_macro(&format_ident!("Score"), &properties);

        let syn_file = syn::parse_file(output.to_string().as_str()).unwrap();
        let pretty = prettyplease::unparse(&syn_file);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn generate_define_mapper_macro_compile_succeeds() {
        let properties = vec![PropertyField {
            ident: format_ident!("score"),
            ty: syn::parse_quote!(i32),
        }];
        let output = generate_define_mapper_macro(&format_ident!("Score"), &properties);

        let mut file = new_trybuild_file!();
        file.write_all(output.to_string().as_bytes()).unwrap();
        file.write_all(
            "
            define_score_mapper!{}
            fn main(){
                let _m = Mapper;
            }"
            .as_bytes(),
        )
        .unwrap();

        let t = trybuild::TestCases::new();
        t.pass(file.path());
    }

    #[test]
    fn generate_define_mapper_macro_generated_slint_typ_snapshot() {
        let properties = vec![PropertyField {
            ident: format_ident!("yesno"),
            ty: parse_quote!(r#YesNo),
        }];
        let output = generate_define_mapper_macro(&format_ident!("Ask"), &properties);

        let syn_file = syn::parse_file(output.to_string().as_str()).unwrap();
        let pretty = prettyplease::unparse(&syn_file);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn generate_state_struct_snapshot() {
        let properties = vec![PropertyField {
            ident: format_ident!("name"),
            ty: parse_quote!(sp::SharedString),
        }];
        let output = generate_state_struct(&format_ident!("Hello"), &properties);

        let syn_file = syn::parse_file(output.to_string().as_str()).unwrap();
        let pretty = prettyplease::unparse(&syn_file);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn generate_mapper_trait_snapshot() {
        let properties = vec![PropertyField {
            ident: format_ident!("name"),
            ty: parse_quote!(sp::SharedString),
        }];
        let output = generate_mapper_trait(&format_ident!("GoodMorning"), &properties);

        let syn_file = syn::parse_file(output.to_string().as_str()).unwrap();
        let pretty = prettyplease::unparse(&syn_file);
        insta::assert_snapshot!(pretty);
    }
}

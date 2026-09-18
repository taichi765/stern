use syn::{Expr, Ident, Token, Type, braced, parse::Parse, punctuated::Punctuated, token::Brace};

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct DefineMapperInput {
    pub base_name: BaseName,
    pub punct: Token![,],
    pub properties: Properties,
}

impl Parse for DefineMapperInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            base_name: input.parse()?,
            punct: input.parse()?,
            properties: input.parse()?,
        })
    }
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct BaseName {
    pub base_name_tok: Ident,
    pub colon: Token![:],
    pub name: Ident,
}

impl Parse for BaseName {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident != "base_name" {
            return Err(syn::Error::new_spanned(ident, "expected 'base_name'"));
        }
        Ok(Self {
            base_name_tok: ident,
            colon: input.parse()?,
            name: input.parse()?,
        })
    }
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct Properties {
    pub properties_ident: Ident,
    pub colon: Token![:],
    pub brace: Brace,
    pub properties: Punctuated<MapperProperty, Token![,]>,
}

impl Parse for Properties {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let properties_ident: Ident = input.parse()?;
        if properties_ident != "properties" {
            return Err(syn::Error::new_spanned(
                properties_ident,
                "expected 'properties'",
            ));
        }
        let colon = input.parse()?;

        let content;
        let brace = braced!(content in input);
        let properties = content.parse_terminated(MapperProperty::parse, Token![,])?;
        Ok(Self {
            properties_ident,
            colon,
            brace,
            properties,
        })
    }
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct MapperProperty {
    pub name: Ident,
    pub colon: Token![:],
    pub brace: Brace,
    pub domain_typ: Option<DomainTypEntry>,
    pub slint_typ: SlintTypEntry,
    pub mapper: Option<MapperEntry>,
}

impl Parse for MapperProperty {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let colon: Token![:] = input.parse()?;

        let content;
        let brace = braced!(content in input);
        let prop_infos = content.parse_terminated(PropertyInfo::parse, Token![,])?;

        let mut domain_typ = None;
        let mut slint_typ = None;
        let mut mapper = None;
        for info in prop_infos {
            match info {
                PropertyInfo::DomainTyp(v) => {
                    if domain_typ.is_some() {
                        return Err(syn::Error::new_spanned(
                            v.ident,
                            "found multiple entry of 'domain_typ'",
                        ));
                    }
                    domain_typ = Some(v)
                }
                PropertyInfo::SlintTyp(v) => {
                    if slint_typ.is_some() {
                        return Err(syn::Error::new_spanned(
                            v.ident,
                            "found multiple entry of 'slint_typ'",
                        ));
                    }
                    slint_typ = Some(v)
                }
                PropertyInfo::Mapper(v) => {
                    if mapper.is_some() {
                        return Err(syn::Error::new_spanned(
                            v.ident,
                            "found multiple entry of 'mapper'",
                        ));
                    }
                    mapper = Some(v)
                }
            }
        }

        if (domain_typ.is_some() && mapper.is_none()) || (domain_typ.is_none() && mapper.is_some())
        {
            return Err(syn::Error::new_spanned(
                name.clone(),
                "only one of 'domain_typ' and 'mapper' is passed as input. you need to specify both of them or none of them.",
            ));
        }

        Ok(Self {
            name: name.clone(),
            colon,
            brace,
            domain_typ: domain_typ,
            slint_typ: slint_typ.ok_or(syn::Error::new_spanned(
                name.clone(),
                "no entry of 'slint_typ' found",
            ))?,
            mapper,
        })
    }
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct DomainTypEntry {
    pub ident: Ident,
    pub colon: Token![:],
    pub typ: Type,
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct SlintTypEntry {
    pub ident: Ident,
    pub colon: Token![:],
    pub typ: Type,
}

#[allow(dead_code)] // may be used in future to report with a span
#[derive(Debug, Clone)]
pub struct MapperEntry {
    pub ident: Ident,
    pub colon: Token![:],
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub enum PropertyInfo {
    DomainTyp(DomainTypEntry),
    SlintTyp(SlintTypEntry),
    Mapper(MapperEntry),
}

impl Parse for PropertyInfo {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        let colon: Token![:] = input.parse()?;
        match ident.to_string().as_str() {
            "domain_typ" => Ok(Self::DomainTyp(DomainTypEntry {
                ident,
                colon,
                typ: input.parse()?,
            })),
            "slint_typ" => Ok(Self::SlintTyp(SlintTypEntry {
                ident,
                colon,
                typ: input.parse()?,
            })),
            "mapper" => Ok(Self::Mapper(MapperEntry {
                ident,
                colon,
                expr: input.parse()?,
            })),
            _ => Err(syn::Error::new_spanned(
                ident,
                "unexpected property info field",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn define_mapper_input_parses_correctly() {
        let got: DefineMapperInput = syn::parse_str(
            "base_name: Score,
            properties:{
            player_name: {
            domain_typ: String, slint_typ: slint::SharedString, mapper: {
            value.to_shared_string()
            }
            },count:{domain_typ: u32, slint_typ:i32,mapper:{value.try_into().unwrap()}}}",
        )
        .unwrap();
        assert_eq!(got.base_name.name.to_string(), "Score");
        assert_eq!(got.properties.properties.len(), 2);
    }

    #[test]
    fn mapped_property_parses_correctly() {
        let got: MapperProperty = syn::parse_str(
            "player_name: {
            domain_typ: String,
            slint_typ: slint::SharedString,
            mapper: {
                value.to_shared_string()
            }
        }",
        )
        .unwrap();
        assert_eq!(got.name.to_string(), "player_name");

        let domain_typ = got.domain_typ.expect("domain_typ should be some");
        assert!(match domain_typ.typ {
            Type::Path(tp) => {
                tp.path == parse_quote!(String)
            }
            _ => false,
        });

        let slint_typ = got.slint_typ.typ;
        assert!(match slint_typ {
            Type::Path(tp) => {
                tp.path == parse_quote!(slint::SharedString)
            }
            _ => false,
        });
    }
}

use quote::format_ident;
use syn::{
    Field, Fields, GenericArgument, Ident, ItemStruct, MetaNameValue, PathArguments, Token, Type,
    TypeTuple, parse::Parse, parse_quote,
};

pub struct RouteMacroAttr {
    pub slint_type_name: Ident,
}

impl Parse for RouteMacroAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let list = input.parse_terminated(MetaNameValue::parse, Token![,])?;
        let mut slint_type = None;
        for meta in list {
            match meta.path {
                p if p.is_ident("slint_type") => {
                    let typ_name: Ident = parse_quote!(meta.value);
                    slint_type = Some(typ_name);
                }
                _ => return Err(syn::Error::new_spanned(meta, "unknown option")),
            }
        }
        Ok(Self {
            slint_type_name: slint_type.unwrap_or(format_ident!("UiNavRoute")),
        })
    }
}

pub struct InnerGlobalComponent {
    pub original_ident: Ident,
    /// `Xxx` if attributed struct's ident was `InnerXxxAdopter`.
    pub name: Ident,
    pub properties: Vec<PropertyField>,
    pub callbacks: Vec<CallbackField>,
    pub callback_trackers: Vec<Field>,
    pub change_trackers: Vec<Field>,
    pub globals: Field,
    pub original: ItemStruct,
}

pub struct PropertyField {
    pub ident: Ident,
    pub ty: Type,
}

pub struct CallbackField {
    pub ident: Ident,
    pub arg_type: TypeTuple,
    pub ret_type: Type,
}

impl Parse for InnerGlobalComponent {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let item_struct: ItemStruct = input.parse()?;
        let name = extract_name(&item_struct.ident)?;

        let Fields::Named(fields) = &item_struct.fields else {
            return Err(syn::Error::new_spanned(
                &item_struct,
                "expected named fields",
            ));
        };

        let mut property_fields = Vec::new();
        let mut callback_fields = Vec::new();
        let mut callback_tracker_fields = Vec::new();
        let mut change_tracker_fields = Vec::new();
        let mut globals = None;
        for f in &fields.named {
            let ident = f.ident.as_ref().unwrap().to_string(); // Named fields always have ident
            if ident.starts_with("callback_tracker_") {
                callback_tracker_fields.push(f.to_owned());
            } else if ident.starts_with("change_tracker") {
                change_tracker_fields.push(f.to_owned());
            } else if let Ok(p) = map_property_field(&f) {
                property_fields.push(p);
            } else if let Ok(cb) = map_callback_field(&f) {
                callback_fields.push(cb);
            } else if f.ident.as_ref().unwrap() == "globals" {
                globals = Some(f.to_owned());
            } else {
                return Err(syn::Error::new_spanned(&f, "unknown kind of field"));
            }
        }
        let Some(globals) = globals else {
            return Err(syn::Error::new_spanned(
                item_struct,
                "cannot find `globals` field",
            ));
        };

        Ok(Self {
            original_ident: item_struct.ident.to_owned(),
            name,
            properties: property_fields,
            callbacks: callback_fields,
            callback_trackers: callback_tracker_fields,
            change_trackers: change_tracker_fields,
            globals,
            original: item_struct,
        })
    }
}

fn map_property_field(f: &Field) -> syn::Result<PropertyField> {
    let syn::Type::Path(ty) = &f.ty else {
        return Err(syn::Error::new_spanned(f, "expected path reference"));
    };
    let mut segs = ty.path.segments.iter();

    // TODO: More readble error message
    let ok = segs.next().map(|seg| seg.ident == "sp");
    if ok.is_none() || ok.unwrap() == false {
        return Err(syn::Error::new_spanned(f, "expected 'sp'"));
    };

    let args = segs
        .next()
        .map(|seg| (seg.ident == "Property").then(|| &seg.arguments));
    let Some(Some(args)) = args else {
        return Err(syn::Error::new_spanned(f, "expected 'Property'"));
    };
    let PathArguments::AngleBracketed(args) = args else {
        return Err(syn::Error::new_spanned(
            f,
            "expected angle bracketed type arguments",
        ));
    };
    let mut args = args.args.iter();
    let ty = args
        .next()
        .ok_or(syn::Error::new_spanned(f, "expected a type argument"))?;
    let GenericArgument::Type(ty) = ty else {
        return Err(syn::Error::new_spanned(f, "expected a type argument"));
    };
    if args.next().is_some() {
        return Err(syn::Error::new_spanned(
            f,
            "expected exactly one type argument, found more",
        ));
    };

    if segs.next().is_some() {
        return Err(syn::Error::new_spanned(
            f,
            "type path should be already end here",
        ));
    };

    return Ok(PropertyField {
        ident: f.ident.clone().unwrap(),
        ty: ty.clone(),
    });
}

fn map_callback_field(f: &Field) -> syn::Result<CallbackField> {
    let syn::Type::Path(ty) = &f.ty else {
        return Err(syn::Error::new_spanned(f, "expected path reference"));
    };
    let mut segs = ty.path.segments.iter();

    let ok = segs.next().map(|seg| seg.ident == "sp");
    if ok.is_none() || ok.unwrap() == false {
        return Err(syn::Error::new_spanned(f, "expected 'sp'"));
    };

    let args = segs
        .next()
        .map(|seg| (seg.ident == "Callback").then(|| &seg.arguments));
    let Some(Some(PathArguments::AngleBracketed(args))) = args else {
        return Err(syn::Error::new_spanned(
            f,
            "expected 'Callback<(Args), Ret>'",
        ));
    };
    let mut args = args.args.iter();
    let Some(GenericArgument::Type(Type::Tuple(cb_params))) = args.next() else {
        return Err(syn::Error::new_spanned(
            f,
            "expected callback's parameter types (tuple)",
        ));
    };
    let Some(GenericArgument::Type(cb_ret)) = args.next() else {
        return Err(syn::Error::new_spanned(
            f,
            "expected callback's return type",
        ));
    };

    if segs.next().is_some() {
        return Err(syn::Error::new_spanned(
            f,
            "type path should be already end here",
        ));
    };

    Ok(CallbackField {
        ident: f.ident.to_owned().unwrap(),
        arg_type: cb_params.to_owned(),
        ret_type: cb_ret.to_owned(),
    })
}

/// Get `Xxx` from `InnerXxxAdopter`
fn extract_name(ident: &Ident) -> syn::Result<Ident> {
    let s = ident.to_string();
    let without_prefix = s.strip_prefix("Inner").ok_or(syn::Error::new_spanned(
        &s,
        "attributed type's ident should match the pattern of 'InnerXxxAdopter'",
    ))?;
    let without_suffix = without_prefix
        .strip_suffix("Adopter")
        .ok_or(syn::Error::new_spanned(
            &s,
            "attributed type's ident should match the pattern of 'InnerXxxAdopter'",
        ))?;
    let original_span = ident.span();
    Ok(format_ident!("{}", without_suffix, span = original_span))
}

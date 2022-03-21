use quote::ToTokens;
// use quote::quote;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{FnArg, ItemFn, ReturnType, Type, Pat, Generics};

pub(crate) fn extract_return_ty(fn_item: &ItemFn) -> Type {
    syn::parse_str(&match &fn_item.sig.output {
        ReturnType::Default => "()".to_string(),
        ReturnType::Type(_, ty) => ty.to_token_stream().to_string(),
    })
    .unwrap()
}

pub(crate) fn fn_args_ty(fn_item: &ItemFn) -> Punctuated<Type, Comma> {
    fn_item.sig.inputs.iter().map(|fn_arg| {
        match fn_arg {
            FnArg::Receiver(_) => todo!(),
            FnArg::Typed(ty) => *ty.ty.clone(),
        }
    }).collect()
}

pub(crate) fn fn_arg_pat(fn_item: &ItemFn) -> Punctuated<Pat, Comma> {
    fn_item.sig.inputs.iter().map(|fn_arg| {
        match fn_arg {
            FnArg::Receiver(_) => todo!(),
            FnArg::Typed(ty) => *ty.pat.clone(),
        }
    }).collect()
}


pub(crate) fn fn_generics(fn_item: &ItemFn) -> Generics {
    fn_item.sig.generics.clone()
}

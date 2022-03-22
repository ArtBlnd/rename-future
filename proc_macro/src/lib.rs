mod checks;
mod extract;

use proc_macro::{self, TokenStream};
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote};
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, Field, Fields, FieldsUnnamed, Ident, ItemFn, ItemStruct,
    ReturnType, Type, Visibility, ItemImpl, ImplItem, GenericParam, TypeTuple, VisPublic
};
use syn::token::Comma;

#[proc_macro_attribute]
pub fn rename_future(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut args = args.into_iter();

    // Extract target name from argument.
    let fut_name: Ident = syn::parse_str(
        &args
            .next()
            .expect("new future name should be specified!")
            .to_string(),
    )
    .unwrap();

    // check it has !Send marker
    let is_unsend = {
        if let Some(marker) = args.next() {
            if marker.to_string() == "(! Send)" {
                true
            } else { 
                false
            }
        } else {
            false
        }
    };

    let fn_item = parse_macro_input!(input as ItemFn);
    if !checks::is_async_fn(&fn_item) {
        // currently, we only supports static async function (not method)
        panic!("target function should be async function");
    }


    // Extract signatures in original function
    // we need them to create custom impl of Future

    let ident = fn_item.sig.ident.clone();
    let ident_org: Ident = quote::format_ident!("__internal_{ident}");

    let def_size_of_fut = crate_size_of_fut_def(&ident_org, &fn_item);
    let def_align_of_fut = crate_align_of_fut_def(&ident_org, &fn_item);
    

    // create stmt that defines new future.
    let def_future = {
        let sof_ident = &def_size_of_fut.sig.ident;
        let aof_ident = &def_align_of_fut.sig.ident;

        let fn_generics_ty: Punctuated<GenericParam, Comma> = extract::fn_generics(&fn_item).params.iter().filter(|v| {
            if let GenericParam::Lifetime(_) = v {
                false
            } else {
                true
            }
        }).cloned().collect();


        let dummy_field: TypeTuple = TypeTuple { 
            paren_token: Default::default(), 
            elems: {
                let mut elems = Punctuated::new();
                elems.push(Type::Verbatim(syn::parse_quote!{
                    [std::mem::MaybeUninit<u8>; #sof_ident::<_, _, #fn_generics_ty>(&#ident_org)]
                }));
                elems.push(Type::Verbatim(syn::parse_quote!{
                    rename_future::Align<{#aof_ident::<_, _, #fn_generics_ty>(&#ident_org)}>
                }));

                // We need to add unsend marker if !Send preset.
                if is_unsend {
                    elems.push(Type::Verbatim(syn::parse_quote!{
                        rename_future::PhantomUnsend
                    }))
                }

                elems
            } 
        };
        
        create_future_struct_def(fut_name, Type::Tuple(dummy_field), &fn_item)
    };

    // Modify original function into inner function
    // which named __internal_~~~~
    let def_org_func = {
        let mut org_func = fn_item.clone();
        org_func.sig.ident = ident_org.clone();
        org_func.vis = Visibility::Inherited;

        org_func
    };

    // Create new signature for new function
    // we are replacing original function with this function signature
    let def_new_func = {
        let mut sig = fn_item.sig.clone();

        // Create return type for new future
        // check we have generics params, if we have generic params, adding it.
        let future_ident = &def_future.ident;
        let generics = &sig.generics.params;
        
        let ty_future: Type = if sig.generics.params.is_empty() {
            syn::parse_quote!(#future_ident)
        } else {
            syn::parse_quote!(#future_ident<#generics>)
        };

        sig.asyncness = None;
        sig.output = ReturnType::Type(
            Default::default(),
            Box::new(ty_future.clone()),
        );

        ItemFn {
            attrs: fn_item.attrs.clone(),
            vis: fn_item.vis.clone(),
            sig,
            block: {
                // Create function body.
                let future_impl_def = create_future_impl_def(&ty_future, &ident_org, &fn_item, is_unsend);
                let drop_impl_def = create_drop_impl_def(&ty_future, &ident_org, &fn_item);
                let fn_pat = extract::fn_arg_pat(&fn_item);

                Box::new(syn::parse_quote!({
                    #future_impl_def
                    #drop_impl_def

                    unsafe { std::mem::transmute(#ident_org(#fn_pat)) }
                }))
            },
        }
    };

    quote! {
        #def_size_of_fut
        #def_align_of_fut
        #def_future
        #def_org_func
        #def_new_func
        
    }.into()
}

// create a returning future type from identifier and async fn arguments
// we need original function arguments to make sure all traits and lifetimes are inherited on new future type.
fn create_future_struct_def(
    ident: Ident,
    dummy_field: Type,
    fn_item: &ItemFn,
) -> ItemStruct {
    let fn_arg_ty = extract::fn_args_ty(fn_item);
    let fn_generics = extract::fn_generics(fn_item);

    // we only need traits and lifetimes so we mark field as PhamtomData
    let fields = {
        let mut params_field = Punctuated::new();

        // adding dummy 
        params_field.push(Field {
            attrs: Vec::new(),
            vis: Visibility::Inherited,
            ident: None,
            colon_token: None,
            ty: dummy_field,
        });

        params_field.push(Field {
            attrs: Vec::new(),
            vis: Visibility::Inherited,
            ident: None,
            colon_token: None,
            ty: syn::parse_quote!(std::marker::PhantomData<(#fn_arg_ty)>),
        });

        params_field.push(Field {
            attrs: Vec::new(),
            vis: Visibility::Inherited,
            ident: None,
            colon_token: None,
            ty: syn::parse_quote!(std::marker::PhantomPinned),
        });

        // Insert all field into struct
        Fields::Unnamed(FieldsUnnamed {
            paren_token: Default::default(),
            unnamed: params_field,
        })
    };

    ItemStruct {
        attrs: Vec::new(),
        vis: Visibility::Public(VisPublic {
            pub_token: Default::default(),
        }),
        struct_token: Default::default(),
        ident: ident.clone(),
        generics: fn_generics,
        fields: fields,
        semi_token: None,
    }
}

fn create_future_impl_def(ty_fut: &Type, ident_org: &Ident, fn_item: &ItemFn, is_unsend: bool) -> ItemImpl {
    let fn_arg_ty = extract::fn_args_ty(fn_item);
    let fn_generics = extract::fn_generics(fn_item);
    let fn_ret = extract::extract_return_ty(&fn_item);

    ItemImpl {
        attrs: Vec::new(),
        defaultness: None,
        unsafety: None,
        impl_token: Default::default(),
        generics: fn_generics.clone(),
        trait_: Some((None, syn::parse_quote!(std::future::Future), Default::default())),
        self_ty: Box::new(syn::parse_quote!(#ty_fut)),
        brace_token: Default::default(),
        items: {
            let fn_arg_ty = fn_arg_ty.clone();
            let fn_generics_ty: Punctuated<GenericParam, Comma> = fn_generics.params.iter().filter(|v| {
                if let GenericParam::Lifetime(_) = v {
                    false
                } else {
                    true
                }
            }).cloned().collect();
            let fn_generics_lt: Punctuated<GenericParam, Comma> = fn_generics.params.iter().filter(|v| {
                if let GenericParam::Lifetime(_) = v {
                    true
                } else {
                    false
                }
            }).cloned().collect();

            let lifetime_token: TokenStream2 = if fn_generics_lt.is_empty() {
                syn::parse_quote!()
            } else {
                syn::parse_quote!(#fn_generics_lt, )
            };

            let send_token: TokenStream2 = if is_unsend {
                syn::parse_quote!()
            } else {
                syn::parse_quote!(+ Send)
            };

            let mut items = Vec::new();
            items.push(syn::parse_quote!{
                type Output = #fn_ret;  
            });
            items.push(ImplItem::Method({
                let call_poll_fn: ItemFn = syn::parse_quote! {
                    fn call_poll<#lifetime_token __T, __Q, __F, #fn_generics_ty>(
                        _: &__T,
                        fut: std::pin::Pin<&mut __F>, 
                        cx: &mut std::task::Context<'_>
                    ) -> std::task::Poll<__F::Output>
                    where
                        __T: Fn(#fn_arg_ty) -> __Q,
                        __Q: std::future::Future<Output = __F::Output> #send_token,
                        __F: std::future::Future,
                    {
                        let fut: std::pin::Pin<&mut __Q> = unsafe { std::mem::transmute(fut) };
                        fut.poll(cx)
                    }
                };

                syn::parse_quote!{
                    fn poll(
                        self: std::pin::Pin<&mut Self>, 
                        cx: &mut std::task::Context<'_>) 
                    -> std::task::Poll<Self::Output> {
                        #call_poll_fn
                        call_poll::<_, _, _, #fn_generics_ty>(&#ident_org, self, cx)
                    }
                } 
            }));

            items
        },
    }
}


fn create_drop_impl_def(ty_fut: &Type, ident_org: &Ident, fn_item: &ItemFn) -> ItemImpl {
    let fn_generics = extract::fn_generics(fn_item);

    ItemImpl {
        attrs: Vec::new(),
        defaultness: None,
        unsafety: None,
        impl_token: Default::default(),
        generics: fn_generics.clone(),
        trait_: Some((None, syn::parse_quote!(Drop), Default::default())),
        self_ty: Box::new(syn::parse_quote!(#ty_fut)),
        brace_token: Default::default(),
        items: {
            let mut items = Vec::new();
            items.push(ImplItem::Method({
                let fn_arg_ty = extract::fn_args_ty(fn_item);
                let fn_generics_ty: Punctuated<GenericParam, Comma> = fn_generics.params.iter().filter(|v| {
                    if let GenericParam::Lifetime(_) = v {
                        false
                    } else {
                        true
                    }
                }).cloned().collect();
                let fn_generics_lt: Punctuated<GenericParam, Comma> = fn_generics.params.iter().filter(|v| {
                    if let GenericParam::Lifetime(_) = v {
                        true
                    } else {
                        false
                    }
                }).cloned().collect();

                let lifetime_token: TokenStream2 = if fn_generics_lt.is_empty() {
                    syn::parse_quote!()
                } else {
                    syn::parse_quote!(#fn_generics_lt, )
                };

                syn::parse_quote!{
                    fn drop(&mut self) {
                        fn call_drop<#lifetime_token __T, __Q, __F, #fn_generics_ty>(
                            _: &__T, 
                            fut: &mut __F
                        )
                        where
                            __T: Fn(#fn_arg_ty) -> __Q,
                        {
                            let fut: &mut __Q = unsafe { std::mem::transmute(fut) };
                            if std::mem::needs_drop::<__Q>() {
                                unsafe {
                                    std::ptr::drop_in_place(fut);
                                }
                            }
                        }
            
                        call_drop::<_, _, _>(&#ident_org, self)
                    }
                }
            }));

            items
        },
    }
}


fn crate_size_of_fut_def(ident_org: &Ident, fn_item: &ItemFn) -> ItemFn {
    let ident: Ident = quote::format_ident!("{ident_org}_sof");

    let fn_generics = extract::fn_generics(fn_item);
    let fn_generics_ty = fn_generics.params;
    let fn_arg_ty = extract::fn_args_ty(fn_item);

    syn::parse_quote! {
        pub const fn #ident<F, Fut, #fn_generics_ty>(_: &F) -> usize
        where
            F: Fn(#fn_arg_ty) -> Fut,
        {
            std::mem::size_of::<Fut>()
        }
    }
}


fn crate_align_of_fut_def(ident_org: &Ident, fn_item: &ItemFn) -> ItemFn {
    let ident: Ident = quote::format_ident!("{ident_org}_aof");

    let fn_generics = extract::fn_generics(fn_item);
    let fn_generics_ty = fn_generics.params;
    let fn_arg_ty = extract::fn_args_ty(fn_item);

    syn::parse_quote! {
        pub const fn #ident<F, Fut, #fn_generics_ty>(_: &F) -> usize
        where
            F: Fn(#fn_arg_ty) -> Fut,
        {
            std::mem::align_of::<Fut>()
        }
    }
}
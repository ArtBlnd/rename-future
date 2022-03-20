use syn::ItemFn;

pub(crate) fn is_async_fn(fn_item: &ItemFn) -> bool {
    // async sugered function
    if fn_item.sig.asyncness.is_some() {
        return true;
    }

    return false;
}

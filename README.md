# rename-future
You can name anonymous Future from async fn without dyn or Box!

# PLEASE READ THIS
THIS PROJECT NOT YET TESTED! DO NOT USE ON PRODUCTION! 

## What is the problem of `async fn` and its returning `Future`?
The return type of `async fn` is anonymous. means, it is really hard to move around `Future` of `async fn` 
unless `type_alias_impl_trait` stabilizes. for example, most `Service` design requires `Future` as associated type.

Simple example with `tower::Service`.
```rust
impl Service<Request> for AsyncFnService {
    type Response = usize;
    type Error = ();
    type Future = impl Future<Output = Result<Self::Response, Self::Error>>; // ERROR! not allowed until `type_alias_impl_trait` stablizes

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        async { 10 }
    }
}
```

To solve this problem, we need trait boxing. which means make extra runtime costs. and bad, long, ugly type signature something like this.  
This makes boxing itself is more expensive then function itself.
```rust
impl Service<Request> for AsyncFnService {
    type Response = usize;
    type Error = ();
    type Future = Pin<Box<dyn Future<Output = Result<Self::response, Self::Error> + Send + 'static>>; // LONG AND UGLY!! also makes vtable and heap allocation!

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        Box::pin(async { 10 })
    }
}
```

## Using rename-future
With rename-future, you can simply define a new name for returning future! without any runtime costs.  
Only you have to do is define a new `async fn` and add attribute.
```rust
impl Service<Request> for AsyncFnService {
    type Response = ();
    type Error = ();
    type Future = FooAsyncFnFuture; // simply use renamed Future! no extra costs!

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        foo()
    }
}

#[rename_future(FooAsyncFnFuture)]
async fn foo() -> usize {
    10
}
```

You also can pass references via adding lifetimes. 
```rust
#[rename_future(FooAsyncFnFuture)]
async fn add_10<'a>(v: &'a usize) -> usize {
    *v + 10
}
```

Lifetime will be always required because new defined `Future` will always require an explicit lifetime.  
The signature of FooAsyncFnFuture will look like this.
```rust
struct FooAsyncFnFuture<'a> {
    /* private fields */
}
```

## How does it work?
We create exact same size and aligned named struct on macro and transmute it.
at the end, when `poll` is called on new named future. `Pin<&mut Self>` is transmutted into original function's return `Pin<&mut {Some_Anon_Original_Future}>`. and original `poll` will be called.
everything will be inlined so it will just work like holding original `Future`. without any costs.

this is original function
```rust
#[rename_future(AsyncFnFuture)]
async fn async_fn() -> usize {
    10
}
```

and this is how its look like after macro expansion!
```rust
pub const fn __internal_async_fn_sof<F, Fut>(_: &F) -> usize
where
    F: Fn() -> Fut,
{
    std::mem::size_of::<Fut>()
}
pub const fn __internal_async_fn_aof<F, Fut>(_: &F) -> usize
where
    F: Fn() -> Fut,
{
    std::mem::align_of::<Fut>()
}
struct AsyncFnFuture(
    (
        [u8; __internal_async_fn_sof::<_, _>(&__internal_async_fn)],
        rename_future::Align<{ __internal_async_fn_aof::<_, _>(&__internal_async_fn) }>,
    ),
    std::marker::PhantomData<()>,
    std::marker::PhantomPinned,
);
async fn __internal_async_fn() -> usize {
    10
}
fn async_fn() -> AsyncFnFuture {
    impl std::future::Future for AsyncFnFuture {
        type Output = usize;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            fn call_poll<__T, __Q, __F>(
                _: &__T,
                fut: std::pin::Pin<&mut __F>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<__F::Output>
            where
                __T: Fn() -> __Q,
                __Q: std::future::Future<Output = __F::Output>,
                __F: std::future::Future,
            {
                let fut: std::pin::Pin<&mut __Q> = unsafe { std::mem::transmute(fut) };
                fut.poll(cx)
            }
            call_poll::<_, _, _>(&__internal_async_fn, self, cx)
        }
    }
    unsafe { std::mem::transmute(__internal_async_fn()) }
}
```

Everything is safe until those conditions.
1. New `Future` has same size, alignment, lifetime, trait as original `Future`
2. New `Future` is always `!Unpin`
3. New `Future` should be transmutted into exact original `Future` that it was when its polled.


## Limitations
Currently, `rename-future` does not support `async fn` with generic types. because current rust compiler cannot eval size or align of type when it has generic types.
you can use it by enabling `generic_const_exprs` nightly feature if you want. but this is not supported on stable version of rust. Also, `rename-future` does not support `impl Trait` return type. supporting `impl Trait` return type means `type_alias_impl_trait` is stabilized! which makes this crate useless.